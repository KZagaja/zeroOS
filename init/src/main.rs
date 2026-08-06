#![cfg_attr(not(target_os = "linux"), allow(dead_code))]

use std::{
    collections::VecDeque,
    env,
    io::{self, Read, Write},
    os::unix::net::UnixStream,
    sync::atomic::{AtomicBool, Ordering},
    thread,
    time::{Duration, Instant},
};

const SOCKET: &str = "/run/zeroos/core-v1.sock";
const MAX_REQUEST: usize = 4096;
const LOG_CAPACITY: usize = 256;
const RESTART_LIMIT: usize = 3;
const RESTART_WINDOW: Duration = Duration::from_secs(10);
#[cfg(target_os = "linux")]
const SHUTDOWN_GRACE: Duration = Duration::from_secs(2);
#[cfg(target_os = "linux")]
const RB_POWER_OFF: i32 = 0x4321_fedc;

static SIGCHLD_PENDING: AtomicBool = AtomicBool::new(false);
static SHUTDOWN_PENDING: AtomicBool = AtomicBool::new(false);

#[cfg(target_os = "linux")]
unsafe extern "C" {
    fn signal(signal: i32, handler: usize) -> usize;
    fn waitpid(pid: i32, status: *mut i32, options: i32) -> i32;
    fn kill(pid: i32, signal: i32) -> i32;
    fn sync();
    fn reboot(command: i32) -> i32;
    fn fork() -> i32;
    fn _exit(status: i32) -> !;
}

#[derive(Clone, Copy)]
struct ServiceDef {
    name: &'static str,
    deps: &'static [&'static str],
}

const SERVICES: &[ServiceDef] = &[
    ServiceDef {
        name: "base",
        deps: &[],
    },
    ServiceDef {
        name: "flaky",
        deps: &["base"],
    },
    ServiceDef {
        name: "dependent",
        deps: &["flaky"],
    },
    ServiceDef {
        name: "independent",
        deps: &[],
    },
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    Stopped,
    Starting,
    Running,
    Stopping,
    Failed,
}

struct Service {
    state: State,
    pid: Option<u32>,
    desired: bool,
    generation: u32,
    started: Option<Instant>,
    failures: VecDeque<Instant>,
    restart: bool,
}

impl Default for Service {
    fn default() -> Self {
        Self {
            state: State::Stopped,
            pid: None,
            desired: false,
            generation: 0,
            started: None,
            failures: VecDeque::new(),
            restart: false,
        }
    }
}

#[derive(Clone)]
struct LogRecord {
    millis: u128,
    level: &'static str,
    component: String,
    event: String,
    message: String,
}

impl LogRecord {
    fn line(&self) -> String {
        format!(
            "{}\t{}\t{}\t{}\t{}",
            self.millis,
            self.level,
            escape(&self.component),
            escape(&self.event),
            escape(&self.message)
        )
    }
}

struct Runtime {
    services: Vec<Service>,
    logs: VecDeque<LogRecord>,
    boot: Instant,
    shutting_down: bool,
    selftest: Option<Selftest>,
}

#[derive(Clone, Copy)]
enum Selftest {
    WaitInitial,
    #[cfg(target_os = "linux")]
    Crash(u8),
    #[cfg(target_os = "linux")]
    Verify,
    #[cfg(target_os = "linux")]
    WaitRecovery,
    #[cfg(target_os = "linux")]
    AwaitLogs,
}

impl Runtime {
    fn new() -> Self {
        Self {
            services: SERVICES.iter().map(|_| Service::default()).collect(),
            logs: VecDeque::with_capacity(LOG_CAPACITY),
            boot: Instant::now(),
            shutting_down: false,
            selftest: None,
        }
    }

    fn log(&mut self, level: &'static str, component: &str, event: &str, message: &str) {
        if self.logs.len() == LOG_CAPACITY {
            self.logs.pop_front();
        }
        let record = LogRecord {
            millis: self.boot.elapsed().as_millis(),
            level,
            component: component.into(),
            event: event.into(),
            message: message.into(),
        };
        println!("{}", record.line());
        let _ = io::stdout().flush();
        self.logs.push_back(record);
    }

    fn index(name: &str) -> Option<usize> {
        SERVICES.iter().position(|service| service.name == name)
    }

    fn start(&mut self, name: &str, administrative: bool) -> Result<(), &'static str> {
        if self.shutting_down {
            return Err("SHUTTING_DOWN");
        }
        let index = Self::index(name).ok_or("NO_SERVICE")?;
        self.want_with_dependencies(index, administrative);
        self.start_ready();
        Ok(())
    }

    fn want_with_dependencies(&mut self, index: usize, administrative: bool) {
        for dependency in SERVICES[index].deps {
            self.want_with_dependencies(
                Self::index(dependency).expect("validated graph"),
                administrative,
            );
        }
        if administrative {
            self.services[index].failures.clear();
            if self.services[index].state == State::Failed {
                self.services[index].state = State::Stopped;
            }
        }
        self.services[index].desired = true;
    }

    fn start_ready(&mut self) {
        loop {
            let next = (0..SERVICES.len()).find(|&index| {
                self.services[index].desired
                    && self.services[index].state == State::Stopped
                    && SERVICES[index].deps.iter().all(|dependency| {
                        self.services[Self::index(dependency).unwrap()].state == State::Running
                    })
            });
            let Some(index) = next else { break };
            if let Err(error) = self.spawn(index) {
                self.services[index].state = State::Failed;
                self.services[index].desired = false;
                self.log(
                    "ERROR",
                    SERVICES[index].name,
                    "spawn-failed",
                    &error.to_string(),
                );
            }
        }
    }

    fn spawn(&mut self, index: usize) -> io::Result<()> {
        self.services[index].generation += 1;
        let generation = self.services[index].generation;
        #[cfg(test)]
        let child_id = 10_000 + generation;
        #[cfg(not(test))]
        let child = std::process::Command::new("/init")
            .args(["--fixture", SERVICES[index].name, &generation.to_string()])
            .spawn()?;
        #[cfg(not(test))]
        let child_id = child.id();
        self.services[index].pid = Some(child_id);
        self.services[index].started = Some(Instant::now());
        self.services[index].state = State::Starting;
        self.log(
            "INFO",
            SERVICES[index].name,
            "started",
            &format!("pid={child_id} generation={generation}"),
        );
        #[cfg(not(test))]
        drop(child);
        Ok(())
    }

    fn ready(&mut self, name: &str, pid: u32) -> Result<(), &'static str> {
        let index = Self::index(name).ok_or("NO_SERVICE")?;
        if self.services[index].pid != Some(pid) || self.services[index].state != State::Starting {
            return Err("STALE_FIXTURE");
        }
        self.services[index].state = State::Running;
        self.log("INFO", name, "ready", &format!("pid={pid}"));
        self.start_ready();
        Ok(())
    }

    fn fixture_log(
        &mut self,
        name: &str,
        pid: u32,
        level: &str,
        event: &str,
        message: &str,
    ) -> Result<(), &'static str> {
        let index = Self::index(name).ok_or("NO_SERVICE")?;
        if self.services[index].pid != Some(pid) {
            return Err("STALE_FIXTURE");
        }
        let level = match level {
            "INFO" => "INFO",
            "WARN" => "WARN",
            "ERROR" => "ERROR",
            _ => return Err("BAD_REQUEST"),
        };
        self.log(level, name, event, message);
        Ok(())
    }

    fn stop(&mut self, name: &str) -> Result<(), &'static str> {
        if self.shutting_down {
            return Err("SHUTTING_DOWN");
        }
        let index = Self::index(name).ok_or("NO_SERVICE")?;
        self.stop_dependents(index);
        Ok(())
    }

    fn stop_dependents(&mut self, index: usize) {
        for dependent in (0..SERVICES.len()).rev() {
            if SERVICES[dependent]
                .deps
                .iter()
                .any(|dependency| *dependency == SERVICES[index].name)
            {
                self.stop_dependents(dependent);
            }
        }
        self.services[index].desired = false;
        self.terminate(index, 15);
    }

    fn terminate(&mut self, index: usize, signal: i32) {
        if let Some(pid) = self.services[index].pid {
            if !self.services[index].desired {
                self.services[index].state = State::Stopping;
            }
            #[cfg(target_os = "linux")]
            unsafe {
                kill(pid as i32, signal);
            }
            self.log(
                "INFO",
                SERVICES[index].name,
                "stop-sent",
                &format!("pid={pid} signal={signal}"),
            );
        } else if self.services[index].state != State::Failed {
            self.services[index].state = State::Stopped;
        }
    }

    fn child_exit(&mut self, pid: u32, now: Instant) {
        let Some(index) = self
            .services
            .iter()
            .position(|service| service.pid == Some(pid))
        else {
            self.log("INFO", "core", "orphan-reaped", &format!("pid={pid}"));
            return;
        };
        let unexpected = self.services[index].desired && !self.shutting_down;
        let restart = self.services[index].restart;
        self.services[index].pid = None;
        self.services[index].started = None;
        self.services[index].state = State::Stopped;
        self.services[index].restart = false;
        self.log(
            if unexpected { "WARN" } else { "INFO" },
            SERVICES[index].name,
            "exited",
            &format!("pid={pid} unexpected={unexpected}"),
        );
        if restart && !self.shutting_down {
            self.services[index].desired = true;
            self.start_ready();
            return;
        }
        if !unexpected {
            return;
        }
        let failures = &mut self.services[index].failures;
        failures.push_back(now);
        while failures
            .front()
            .is_some_and(|failure| now.duration_since(*failure) > RESTART_WINDOW)
        {
            failures.pop_front();
        }
        if failures.len() > RESTART_LIMIT {
            self.services[index].state = State::Failed;
            self.services[index].desired = false;
            self.log(
                "ERROR",
                SERVICES[index].name,
                "permanent-failure",
                "restart-limit=3 window-seconds=10",
            );
            self.fail_dependents(index);
        } else {
            self.start_ready();
        }
    }

    fn fail_dependents(&mut self, index: usize) {
        for dependent in (0..SERVICES.len()).rev() {
            if SERVICES[dependent]
                .deps
                .iter()
                .any(|dependency| *dependency == SERVICES[index].name)
            {
                self.fail_dependents(dependent);
                self.services[dependent].desired = false;
                self.terminate(dependent, 15);
            }
        }
    }

    fn reset_healthy_budgets(&mut self, now: Instant) {
        for service in &mut self.services {
            if !service.failures.is_empty()
                && service
                    .started
                    .is_some_and(|started| now.duration_since(started) >= RESTART_WINDOW)
                && service.state == State::Running
            {
                service.failures.clear();
            }
        }
    }

    fn restart(&mut self, name: &str) -> Result<(), &'static str> {
        let index = Self::index(name).ok_or("NO_SERVICE")?;
        if self.shutting_down {
            return Err("SHUTTING_DOWN");
        }
        self.services[index].failures.clear();
        for affected in (0..SERVICES.len()).rev() {
            if affected == index
                || (self.services[affected].desired && Self::depends_on(affected, index))
            {
                self.services[affected].desired = false;
                if self.services[affected].pid.is_some() {
                    self.services[affected].restart = true;
                    self.terminate(affected, 15);
                } else {
                    self.services[affected].restart = false;
                    self.services[affected].desired = true;
                    if self.services[affected].state == State::Failed {
                        self.services[affected].state = State::Stopped;
                    }
                }
            }
        }
        self.start_ready();
        Ok(())
    }

    fn depends_on(service: usize, dependency: usize) -> bool {
        SERVICES[service].deps.iter().any(|name| {
            let direct = Self::index(name).unwrap();
            direct == dependency || Self::depends_on(direct, dependency)
        })
    }

    fn status(&self) -> String {
        SERVICES
            .iter()
            .enumerate()
            .map(|(index, definition)| {
                format!(
                    "{}={:?}:pid={}",
                    definition.name,
                    self.services[index].state,
                    self.services[index].pid.map_or(0, |pid| pid)
                )
            })
            .collect::<Vec<_>>()
            .join(" ")
    }

    fn log_text(&self) -> String {
        self.logs
            .iter()
            .map(LogRecord::line)
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn dispatch(&mut self, request: &str) -> String {
        let words: Vec<_> = request.split_whitespace().collect();
        if words.first() != Some(&"ZEROOS/1") {
            return "ERR ZEROOS/1 UNSUPPORTED_VERSION supported=1".into();
        }
        let result = match words.as_slice() {
            [_, "STATUS"] => return format!("OK ZEROOS/1 STATUS {}", self.status()),
            [_, "LOGS"] => {
                return format!(
                    "OK ZEROOS/1 LOGS count={}\n{}",
                    self.logs.len(),
                    self.log_text()
                );
            }
            [_, "START", name] => self.start(name, true),
            [_, "STOP", name] => self.stop(name),
            [_, "RESTART", name] => self.restart(name),
            [_, "SHUTDOWN"] => {
                if self.shutting_down {
                    Err("SHUTTING_DOWN")
                } else {
                    self.shutting_down = true;
                    Ok(())
                }
            }
            [_, "FIXTURE", "READY", name, pid] => pid
                .parse()
                .map_err(|_| "BAD_REQUEST")
                .and_then(|pid| self.ready(name, pid)),
            [_, "FIXTURE", "LOG", name, pid, level, event, message @ ..] if !message.is_empty() => {
                pid.parse()
                    .map_err(|_| "BAD_REQUEST")
                    .and_then(|pid| self.fixture_log(name, pid, level, event, &message.join(" ")))
            }
            _ => Err("BAD_REQUEST"),
        };
        match result {
            Ok(()) => "OK ZEROOS/1".into(),
            Err(code) => format!("ERR ZEROOS/1 {code}"),
        }
    }

    fn console(&mut self, line: &str) -> String {
        let words: Vec<_> = line.split_whitespace().collect();
        match words.as_slice() {
            ["help"] => "help status logs start <service> stop <service> restart <service> api-version selftest shutdown".into(),
            ["status"] => self.dispatch("ZEROOS/1 STATUS"),
            ["logs"] => {
                let response = self.dispatch("ZEROOS/1 LOGS");
                #[cfg(target_os = "linux")]
                if matches!(self.selftest, Some(Selftest::AwaitLogs)) {
                    self.log("INFO", "selftest", "logs-retrieved", "pass");
                    #[cfg(target_os = "linux")]
                    unsafe {
                        kill(1, 15);
                    }
                }
                response
            }
            ["start", name] => self.dispatch(&format!("ZEROOS/1 START {name}")),
            ["stop", name] => self.dispatch(&format!("ZEROOS/1 STOP {name}")),
            ["restart", name] => self.dispatch(&format!("ZEROOS/1 RESTART {name}")),
            ["api-version"] => "ZEROOS/1 socket=/run/zeroos/core-v1.sock".into(),
            ["shutdown"] => self.dispatch("ZEROOS/1 SHUTDOWN"),
            ["selftest"] if self.selftest.is_none() && !self.shutting_down => {
                for name in ["base", "flaky", "dependent", "independent"] {
                    let _ = self.start(name, true);
                }
                self.spawn_orphan();
                self.selftest = Some(Selftest::WaitInitial);
                "OK ZEROOS/1 SELFTEST started".into()
            }
            ["selftest"] => "ERR ZEROOS/1 BUSY".into(),
            [] => String::new(),
            _ => "ERR ZEROOS/1 BAD_COMMAND".into(),
        }
    }

    fn spawn_orphan(&mut self) {
        #[cfg(target_os = "linux")]
        let _ = std::process::Command::new("/init")
            .arg("--fixture-orphan")
            .spawn();
    }

    #[cfg(target_os = "linux")]
    fn advance_selftest(&mut self) {
        let Some(stage) = self.selftest else { return };
        let flaky = Self::index("flaky").unwrap();
        let dependent = Self::index("dependent").unwrap();
        let independent = Self::index("independent").unwrap();
        match stage {
            Selftest::WaitInitial
                if self.services[flaky].generation >= 2
                    && self.services[flaky].state == State::Running
                    && self.services[dependent].state == State::Running
                    && self.services[independent].state == State::Running =>
            {
                self.log("INFO", "selftest", "restart-before-dependent", "pass");
                self.terminate(flaky, 9);
                self.selftest = Some(Selftest::Crash(1));
            }
            Selftest::Crash(count) if self.services[flaky].state == State::Running => {
                self.terminate(flaky, 9);
                self.selftest = Some(if count == 2 {
                    Selftest::Verify
                } else {
                    Selftest::Crash(count + 1)
                });
            }
            Selftest::Verify
                if self.services[flaky].state == State::Failed
                    && self.services[dependent].pid.is_none()
                    && self
                        .logs
                        .iter()
                        .any(|record| record.event == "orphan-reaped") =>
            {
                let before = self.status();
                let rejected = self.dispatch("ZEROOS/2 STOP independent");
                let isolated =
                    self.services[independent].state == State::Running && before == self.status();
                self.log(
                    "INFO",
                    "selftest",
                    "v2-rejected",
                    &format!("{} unchanged={}", rejected, before == self.status()),
                );
                self.log(
                    "INFO",
                    "selftest",
                    "failure-isolation",
                    &format!("independent-running={isolated}"),
                );
                let _ = self.restart("flaky");
                let _ = self.start("dependent", true);
                self.selftest = Some(Selftest::WaitRecovery);
            }
            Selftest::WaitRecovery
                if self.services[flaky].state == State::Running
                    && self.services[dependent].state == State::Running =>
            {
                self.log("INFO", "selftest", "administrative-recovery", "pass");
                println!("SELFTEST PASS");
                let _ = io::stdout().flush();
                self.selftest = Some(Selftest::AwaitLogs);
            }
            _ => {}
        }
    }

    #[cfg(target_os = "linux")]
    fn shutdown(&mut self) {
        self.shutting_down = true;
        self.log("INFO", "core", "shutdown-started", "grace-ms=2000");
        for index in (0..SERVICES.len()).rev() {
            self.services[index].desired = false;
            self.terminate(index, 15);
        }
        let deadline = Instant::now() + SHUTDOWN_GRACE;
        while Instant::now() < deadline && self.services.iter().any(|service| service.pid.is_some())
        {
            self.reap();
            thread::sleep(Duration::from_millis(10));
        }
        for index in (0..SERVICES.len()).rev() {
            self.terminate(index, 9);
        }
        while self.services.iter().any(|service| service.pid.is_some()) {
            self.reap();
            thread::sleep(Duration::from_millis(10));
        }
        self.reap();
        #[cfg(target_os = "linux")]
        unsafe {
            sync();
        }
        self.log("INFO", "core", "shutdown-complete", "state-synced=true");
    }

    #[cfg(target_os = "linux")]
    fn reap(&mut self) {
        loop {
            let mut status = 0;
            let pid = unsafe { waitpid(-1, &mut status, 1) };
            if pid <= 0 {
                break;
            }
            self.child_exit(pid as u32, Instant::now());
        }
    }
}

fn escape(input: &str) -> String {
    input
        .replace('\\', "\\\\")
        .replace('\t', "\\t")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
}

extern "C" fn on_sigchld(_: i32) {
    SIGCHLD_PENDING.store(true, Ordering::Relaxed);
}

extern "C" fn on_shutdown(_: i32) {
    SHUTDOWN_PENDING.store(true, Ordering::Relaxed);
}

#[cfg(target_os = "linux")]
fn install_signals() {
    unsafe {
        signal(17, on_sigchld as *const () as usize);
        signal(15, on_shutdown as *const () as usize);
        signal(2, on_shutdown as *const () as usize);
    }
}

fn fixture(name: &str, generation: u32) -> ! {
    if name == "flaky" && generation == 1 {
        std::process::exit(1);
    }
    for _ in 0..100 {
        if let Ok(mut socket) = UnixStream::connect(SOCKET) {
            let _ = writeln!(
                socket,
                "ZEROOS/1 FIXTURE READY {name} {}",
                std::process::id()
            );
            let mut response = String::new();
            let _ = socket.read_to_string(&mut response);
            if response.starts_with("OK ZEROOS/1") {
                if let Ok(mut socket) = UnixStream::connect(SOCKET) {
                    let _ = writeln!(
                        socket,
                        "ZEROOS/1 FIXTURE LOG {name} {} INFO fixture-online generation={generation}",
                        std::process::id()
                    );
                }
                loop {
                    thread::park_timeout(Duration::from_secs(60));
                }
            }
        }
        thread::sleep(Duration::from_millis(10));
    }
    std::process::exit(2)
}

#[cfg(target_os = "linux")]
fn fixture_orphan() -> ! {
    unsafe {
        if fork() == 0 {
            thread::sleep(Duration::from_millis(20));
            _exit(0);
        }
        _exit(0)
    }
}

fn read_request(mut stream: &UnixStream) -> Option<String> {
    let _ = stream.set_read_timeout(Some(Duration::from_millis(50)));
    let mut bytes = Vec::new();
    let mut byte = [0];
    while bytes.len() <= MAX_REQUEST {
        match stream.read(&mut byte) {
            Ok(1) if byte[0] == b'\n' => return String::from_utf8(bytes).ok(),
            Ok(1) => bytes.push(byte[0]),
            _ => return None,
        }
    }
    None
}

#[cfg(target_os = "linux")]
fn serve(runtime: &mut Runtime, listener: &std::os::unix::net::UnixListener) {
    loop {
        let Ok((mut stream, _)) = listener.accept() else {
            break;
        };
        if let Some(request) = read_request(&stream) {
            let _ = writeln!(
                stream,
                "{}",
                runtime.dispatch(request.trim_end_matches('\r'))
            );
        }
    }
}

#[cfg(target_os = "linux")]
fn pid1() -> io::Result<()> {
    install_signals();
    std::fs::create_dir_all("/run/zeroos")?;
    if std::path::Path::new(SOCKET).exists() {
        std::fs::remove_file(SOCKET)?;
    }
    let listener = std::os::unix::net::UnixListener::bind(SOCKET)?;
    std::fs::set_permissions(SOCKET, std::os::unix::fs::PermissionsExt::from_mode(0o600))?;
    listener.set_nonblocking(true)?;
    let mut runtime = Runtime::new();
    runtime.log(
        "INFO",
        "core",
        "api-ready",
        "version=1 socket=/run/zeroos/core-v1.sock mode=0600",
    );
    println!("zeroOS init: READY");
    print!("zeroos recovery> ");
    io::stdout().flush()?;

    let (lines_tx, lines_rx) = std::sync::mpsc::channel();
    thread::spawn(move || {
        for line in io::stdin().lines().map_while(Result::ok) {
            let _ = lines_tx.send(line);
        }
    });

    // ponytail: polling is enough for M2's fixed service count; move signals and I/O to epoll/eventfd when runtime load warrants it.
    while !runtime.shutting_down {
        if SIGCHLD_PENDING.swap(false, Ordering::Relaxed) {
            runtime.reap();
        }
        if SHUTDOWN_PENDING.swap(false, Ordering::Relaxed) {
            runtime.shutting_down = true;
            break;
        }
        serve(&mut runtime, &listener);
        while let Ok(line) = lines_rx.try_recv() {
            let response = runtime.console(&line);
            if !response.is_empty() {
                println!("{response}");
            }
            print!("zeroos recovery> ");
            io::stdout().flush()?;
        }
        runtime.reset_healthy_budgets(Instant::now());
        runtime.advance_selftest();
        thread::sleep(Duration::from_millis(10));
    }
    runtime.shutdown();
    unsafe {
        if reboot(RB_POWER_OFF) == 0 {
            return Ok(());
        }
    }
    Err(io::Error::last_os_error())
}

fn main() {
    let args: Vec<_> = env::args().skip(1).collect();
    match args.as_slice() {
        [flag, name, generation] if flag == "--fixture" => {
            fixture(name, generation.parse().unwrap_or(0))
        }
        #[cfg(target_os = "linux")]
        [flag] if flag == "--fixture-orphan" => fixture_orphan(),
        _ => {
            #[cfg(target_os = "linux")]
            if let Err(error) = pid1() {
                eprintln!("zeroOS init: fatal: {error}");
                loop {
                    thread::park();
                }
            }
            #[cfg(not(target_os = "linux"))]
            eprintln!("zeroOS init runs only on Linux");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn graph_and_orders_are_valid() {
        for (index, service) in SERVICES.iter().enumerate() {
            for dependency in service.deps {
                assert!(Runtime::index(dependency).unwrap() < index);
            }
        }
        assert_eq!(
            SERVICES.iter().map(|s| s.name).collect::<Vec<_>>(),
            ["base", "flaky", "dependent", "independent"]
        );
        assert_eq!(
            SERVICES.iter().rev().map(|s| s.name).collect::<Vec<_>>(),
            ["independent", "dependent", "flaky", "base"]
        );
    }

    #[test]
    fn restart_window_exhausts_and_healthy_runtime_resets() {
        let mut runtime = Runtime::new();
        let index = Runtime::index("flaky").unwrap();
        runtime.services[index].desired = true;
        let now = Instant::now();
        for pid in 1..=4 {
            runtime.services[index].pid = Some(pid);
            runtime.services[index].state = State::Running;
            runtime.child_exit(pid, now + Duration::from_millis(pid as u64));
        }
        assert_eq!(runtime.services[index].state, State::Failed);
        runtime.services[index].state = State::Running;
        runtime.services[index].started = Some(now);
        runtime.services[index].failures.push_back(now);
        runtime.reset_healthy_budgets(now + RESTART_WINDOW);
        assert!(runtime.services[index].failures.is_empty());
    }

    #[test]
    fn dependency_failure_isolated() {
        let mut runtime = Runtime::new();
        let flaky = Runtime::index("flaky").unwrap();
        let dependent = Runtime::index("dependent").unwrap();
        let independent = Runtime::index("independent").unwrap();
        runtime.services[dependent].desired = true;
        runtime.services[dependent].state = State::Running;
        runtime.services[dependent].pid = Some(20);
        runtime.services[independent].desired = true;
        runtime.services[independent].state = State::Running;
        runtime.services[independent].pid = Some(30);
        runtime.fail_dependents(flaky);
        assert!(!runtime.services[dependent].desired);
        assert_eq!(runtime.services[independent].state, State::Running);
    }

    #[test]
    fn administrative_start_recovers_dependencies_and_restart_stops_consumers_first() {
        let mut runtime = Runtime::new();
        let base = Runtime::index("base").unwrap();
        let flaky = Runtime::index("flaky").unwrap();
        let dependent = Runtime::index("dependent").unwrap();
        runtime.services[flaky].state = State::Failed;
        runtime.services[flaky].failures.push_back(Instant::now());
        runtime.start("dependent", true).unwrap();
        assert!(runtime.services[base].desired);
        assert_eq!(runtime.services[flaky].state, State::Stopped);
        assert!(runtime.services[flaky].failures.is_empty());

        for (index, pid) in [(flaky, 20), (dependent, 30)] {
            runtime.services[index].state = State::Running;
            runtime.services[index].pid = Some(pid);
            runtime.services[index].desired = true;
        }
        runtime.restart("flaky").unwrap();
        assert_eq!(runtime.services[dependent].state, State::Stopping);
        assert_eq!(runtime.services[flaky].state, State::Stopping);
        let stops: Vec<_> = runtime
            .logs
            .iter()
            .filter(|record| record.event == "stop-sent")
            .map(|record| record.component.as_str())
            .collect();
        assert_eq!(stops, ["dependent", "flaky"]);
    }

    #[test]
    fn logs_retain_and_escape() {
        let mut runtime = Runtime::new();
        for number in 0..=LOG_CAPACITY {
            runtime.log("INFO", "test", "record", &format!("{number}\tline\n\\"));
        }
        assert_eq!(runtime.logs.len(), LOG_CAPACITY);
        assert!(!runtime.log_text().contains("\tline\n"));
        assert!(runtime.log_text().contains("256\\tline\\n\\\\"));
    }

    #[test]
    fn protocol_rejects_versions_and_bad_limits_without_mutation() {
        let mut runtime = Runtime::new();
        let before = runtime.status();
        assert!(
            runtime
                .dispatch("ZEROOS/2 START base")
                .contains("UNSUPPORTED_VERSION")
        );
        assert_eq!(runtime.status(), before);
        assert!(runtime.dispatch("ZEROOS/1 UNKNOWN").contains("BAD_REQUEST"));
        let base = Runtime::index("base").unwrap();
        runtime.services[base].pid = Some(42);
        assert_eq!(
            runtime.dispatch("ZEROOS/1 FIXTURE LOG base 42 INFO fixture-event hello world"),
            "OK ZEROOS/1"
        );
        assert!(runtime.log_text().contains("fixture-event\thello world"));
        let (mut sender, receiver) = UnixStream::pair().unwrap();
        sender.write_all(&vec![b'x'; MAX_REQUEST + 1]).unwrap();
        drop(sender);
        assert!(read_request(&receiver).is_none());
    }

    #[test]
    fn signal_flags_transition_runtime() {
        SIGCHLD_PENDING.store(false, Ordering::Relaxed);
        SHUTDOWN_PENDING.store(false, Ordering::Relaxed);
        on_sigchld(17);
        on_shutdown(15);
        assert!(SIGCHLD_PENDING.swap(false, Ordering::Relaxed));
        assert!(SHUTDOWN_PENDING.swap(false, Ordering::Relaxed));
    }

    #[test]
    fn console_only_dispatches_declared_commands() {
        let mut runtime = Runtime::new();
        assert!(runtime.console("api-version").starts_with("ZEROOS/1"));
        assert!(runtime.console("echo unsafe").contains("BAD_COMMAND"));
        assert!(runtime.console("status | shutdown").contains("BAD_COMMAND"));
    }
}
