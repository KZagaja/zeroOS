#![cfg_attr(not(target_os = "linux"), allow(dead_code))]

#[cfg(target_os = "linux")]
use std::ffi::{c_char, c_void};
use std::{
    collections::VecDeque,
    env, fs,
    io::{self, Read, Write},
    os::unix::net::UnixStream,
    path::PathBuf,
    sync::atomic::{AtomicBool, Ordering},
    thread,
    time::{Duration, Instant},
};
use zeroos_storage::{BootState, Slot};

const SOCKET: &str = "/run/zeroos/core-v1.sock";
const MAX_REQUEST: usize = 4096;
const LOG_CAPACITY: usize = 256;
const RESTART_LIMIT: usize = 3;
const RESTART_WINDOW: Duration = Duration::from_secs(10);
#[cfg(target_os = "linux")]
const SHUTDOWN_GRACE: Duration = Duration::from_secs(2);
#[cfg(target_os = "linux")]
const RB_POWER_OFF: i32 = 0x4321_fedc;
#[cfg(target_os = "linux")]
const RB_AUTOBOOT: i32 = 0x0123_4567;

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
    fn mount(
        source: *const c_char,
        target: *const c_char,
        filesystem: *const c_char,
        flags: usize,
        data: *const c_void,
    ) -> i32;
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
    boot_state: BootState,
    update: &'static str,
    data: &'static str,
    healthy_since: Option<Instant>,
    update_pid: Option<u32>,
    update_install: bool,
    update_progress: PathBuf,
    reboot_after_shutdown: bool,
    recovery_pid: Option<u32>,
    recovery_mutation: Option<RecoveryMutation>,
}

#[derive(Clone, Copy)]
enum RecoveryMutation {
    RepairData,
    FactoryReset,
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
            boot_state: BootState::default(),
            update: "idle",
            data: "locked",
            healthy_since: None,
            update_pid: None,
            update_install: false,
            update_progress: PathBuf::from("/run/zeroos/update-progress"),
            reboot_after_shutdown: false,
            recovery_pid: None,
            recovery_mutation: None,
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
        self.want_with_dependencies(index, administrative)?;
        self.start_ready();
        Ok(())
    }

    fn want_with_dependencies(
        &mut self,
        index: usize,
        administrative: bool,
    ) -> Result<(), &'static str> {
        for dependency in SERVICES[index].deps {
            let dependency = Self::index(dependency).ok_or("INVALID_SERVICE_GRAPH")?;
            self.want_with_dependencies(dependency, administrative)?;
        }
        if administrative {
            self.services[index].failures.clear();
            if self.services[index].state == State::Failed {
                self.services[index].state = State::Stopped;
            }
        }
        self.services[index].desired = true;
        Ok(())
    }

    fn start_ready(&mut self) {
        loop {
            let next = (0..SERVICES.len()).find(|&index| {
                self.services[index].desired
                    && self.services[index].state == State::Stopped
                    && SERVICES[index].deps.iter().all(|dependency| {
                        Self::index(dependency)
                            .is_some_and(|index| self.services[index].state == State::Running)
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
            #[cfg(all(target_os = "linux", not(test)))]
            // SAFETY: `pid` came from `Child::id` for this owned service; `kill` only reads the
            // scalar PID and signal. No pointers, initialization, aliasing, alignment, borrowed
            // lifetime, or thread-shared memory cross the ABI. Failure leaves ownership and later
            // reaping unchanged, so partial failure cannot leak a Rust resource.
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

    fn child_exit(&mut self, pid: u32, success: bool, now: Instant) {
        if self.update_pid == Some(pid) {
            self.update_pid = None;
            self.finish_update(success);
            return;
        }
        if self.recovery_pid == Some(pid) {
            self.recovery_pid = None;
            self.finish_recovery_mutation(success);
            return;
        }
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
            Self::index(name)
                .is_some_and(|direct| direct == dependency || Self::depends_on(direct, dependency))
        })
    }

    fn status(&self) -> String {
        let services = SERVICES
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
            .join(" ");
        format!(
            "{services} mode={} slot={} confirmed={} pending={} sequence={} update={} data={}",
            if self.boot_state.booting == Some(Slot::Recovery) {
                "recovery"
            } else {
                "normal"
            },
            self.boot_state
                .booting
                .unwrap_or(self.boot_state.confirmed)
                .name(),
            self.boot_state.confirmed.name(),
            self.boot_state.pending.map_or("none", Slot::name),
            self.boot_state.sequence,
            self.update,
            self.data,
        )
    }

    fn update(&mut self, install: bool) -> Result<(), &'static str> {
        if self.shutting_down {
            return Err("SHUTTING_DOWN");
        }
        if self.update_pid.is_some() || self.recovery_pid.is_some() {
            return Err("BUSY");
        }
        #[cfg(test)]
        {
            self.update = if install { "staged" } else { "available" };
            if install {
                let sequence = self.boot_state.sequence.checked_add(1).ok_or("DOWNGRADE")?;
                self.boot_state
                    .stage(self.boot_state.confirmed.other(), sequence)?;
            }
            Ok(())
        }
        #[cfg(all(not(test), target_os = "linux"))]
        {
            let inactive = self.boot_state.confirmed.other();
            let mut command = std::process::Command::new("/zeroos-update");
            if !install {
                command.arg("--check");
            }
            command.env("ZEROOS_SEQUENCE", self.boot_state.sequence.to_string());
            if install {
                let label = format!("ZEROOS-{}", inactive.name().to_ascii_uppercase());
                let path = partition_device(&label).ok_or("NO_SLOT")?;
                command.env("ZEROOS_INACTIVE_SLOT", path);
                command.env("ZEROOS_INACTIVE_SLOT_NAME", label);
            }
            let progress = fs::File::create(&self.update_progress).map_err(|_| "UPDATE_FAILED")?;
            let errors = progress.try_clone().map_err(|_| "UPDATE_FAILED")?;
            let child = command
                .stdout(std::process::Stdio::from(progress))
                .stderr(std::process::Stdio::from(errors))
                .spawn()
                .map_err(|_| "UPDATE_FAILED")?;
            self.update_pid = Some(child.id());
            self.update_install = install;
            self.update = "running";
            drop(child);
            Ok(())
        }
        #[cfg(all(not(test), not(target_os = "linux")))]
        {
            let _ = install;
            Err("UNSUPPORTED_PLATFORM")
        }
    }

    fn finish_update(&mut self, success: bool) {
        let output = fs::File::open(&self.update_progress).and_then(|file| {
            let mut text = String::new();
            file.take(16 * 1024 + 1).read_to_string(&mut text)?;
            if text.len() > 16 * 1024 {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "progress too large",
                ));
            }
            Ok(text)
        });
        let sequence = output
            .as_ref()
            .ok()
            .and_then(|text| update_sequence(text, self.update_install));
        #[cfg(feature = "acceptance")]
        if let Ok(text) = &output {
            print!("{text}");
            let _ = io::stdout().flush();
        }
        if !success || sequence.is_none() {
            self.update = "failed";
            return;
        }
        if !self.update_install {
            self.update = "available";
            return;
        }
        let mut staged = self.boot_state;
        let result = match sequence
            .ok_or("UPDATE_FAILED")
            .and_then(|sequence| staged.stage(staged.confirmed.other(), sequence))
        {
            Ok(()) => {
                #[cfg(feature = "acceptance")]
                accept("before-journal-switch");
                #[cfg(target_os = "linux")]
                let persisted =
                    partition_device("ZEROOS-STATE")
                        .ok_or("NO_STATE")
                        .and_then(|journal| {
                            zeroos_storage::write_journal(&journal, &staged)
                                .map_err(|_| "STATE_WRITE_FAILED")
                        });
                #[cfg(not(target_os = "linux"))]
                let persisted = Ok(());
                persisted
            }
            Err(error) => Err(error),
        };
        if result.is_ok() {
            #[cfg(feature = "acceptance")]
            accept("after-journal-switch");
            self.boot_state = staged;
            self.update = "staged";
            self.reboot_after_shutdown = true;
            self.shutting_down = true;
        } else {
            self.update = "failed";
        }
    }

    fn confirm_health(&mut self, now: Instant) {
        let healthy = self.data == "mounted"
            && self
                .services
                .iter()
                .all(|service| !service.desired || service.state == State::Running);
        if !healthy {
            self.healthy_since = None;
            return;
        }
        let since = self.healthy_since.get_or_insert(now);
        if now.duration_since(*since) < Duration::from_secs(10) {
            return;
        }
        #[cfg(feature = "acceptance")]
        accept("before-health-confirmation");
        let mut confirmed = self.boot_state;
        if confirmed.confirm().is_ok() {
            #[cfg(all(not(test), target_os = "linux"))]
            let persisted = partition_device("ZEROOS-STATE")
                .is_some_and(|journal| zeroos_storage::write_journal(&journal, &confirmed).is_ok());
            #[cfg(any(test, not(target_os = "linux")))]
            let persisted = true;
            if persisted {
                #[cfg(feature = "acceptance")]
                accept("after-health-confirmation");
                self.boot_state = confirmed;
                self.healthy_since = None;
            }
            return;
        }
        self.healthy_since = None;
    }

    fn repair_boot(&mut self) -> String {
        #[cfg(feature = "acceptance")]
        accept("before-repair-boot");
        let remain_in_recovery = self.boot_state.booting == Some(Slot::Recovery);
        let confirmed = self.boot_state.confirmed;
        let mut repaired = self.boot_state;
        if repaired.repair(confirmed).is_err() {
            return "ERR ZEROOS/1 REPAIR_FAILED".into();
        }
        #[cfg(all(target_os = "linux", not(test)))]
        {
            let Some(journal) = partition_device("ZEROOS-STATE") else {
                return "ERR ZEROOS/1 NO_STATE".into();
            };
            match zeroos_storage::reconstruct_journal(&journal, &repaired) {
                Ok(state) => self.boot_state = state,
                Err(_) => return "ERR ZEROOS/1 REPAIR_FAILED".into(),
            }
        }
        #[cfg(any(not(target_os = "linux"), test))]
        {
            self.boot_state = repaired;
        }
        if remain_in_recovery {
            self.boot_state.booting = Some(Slot::Recovery);
        }
        #[cfg(feature = "acceptance")]
        accept("after-repair-boot");
        "OK ZEROOS/1".into()
    }

    fn recovery_busy(&self) -> bool {
        self.update_pid.is_some() || self.recovery_pid.is_some() || self.shutting_down
    }

    fn repair_data(&mut self) -> String {
        if self.recovery_busy() {
            return "ERR ZEROOS/1 BUSY".into();
        }
        #[cfg(target_os = "linux")]
        {
            #[cfg(feature = "acceptance")]
            accept("before-repair-data");
            if unmount_data().is_err() || set_stdin_nonblocking(false).is_err() {
                return "ERR ZEROOS/1 UNMOUNT_FAILED".into();
            }
            match std::process::Command::new("/zeroos-data")
                .args(["repair", "/dev/mapper/zeroos-data"])
                .spawn()
            {
                Ok(child) => {
                    self.recovery_pid = Some(child.id());
                    self.recovery_mutation = Some(RecoveryMutation::RepairData);
                    self.data = "repairing";
                    drop(child);
                    "OK ZEROOS/1 REPAIR_DATA started".into()
                }
                Err(_) => "ERR ZEROOS/1 REPAIR_FAILED".into(),
            }
        }
        #[cfg(not(target_os = "linux"))]
        "ERR ZEROOS/1 UNSUPPORTED_PLATFORM".into()
    }

    fn factory_reset(&mut self, confirmation: &str) -> String {
        if confirmation != "ERASE-USER-DATA" {
            return "ERR ZEROOS/1 CONFIRMATION_REQUIRED literal=ERASE-USER-DATA".into();
        }
        if self.recovery_busy() {
            return "ERR ZEROOS/1 BUSY".into();
        }
        #[cfg(target_os = "linux")]
        {
            #[cfg(feature = "acceptance")]
            accept("before-factory-reset");
            let Some(device) = partition_device("ZEROOS-DATA") else {
                return "ERR ZEROOS/1 NO_DATA".into();
            };
            if unmount_data().is_err() {
                return "ERR ZEROOS/1 UNMOUNT_FAILED".into();
            }
            match std::process::Command::new("/zeroos-data")
                .arg("reset")
                .arg(device)
                .arg(confirmation)
                .spawn()
            {
                Ok(child) => {
                    self.recovery_pid = Some(child.id());
                    self.recovery_mutation = Some(RecoveryMutation::FactoryReset);
                    self.data = "resetting";
                    drop(child);
                    "OK ZEROOS/1 FACTORY_RESET confirm=ERASE-USER-DATA".into()
                }
                Err(_) => {
                    let _ = set_stdin_nonblocking(true);
                    "ERR ZEROOS/1 RESET_FAILED".into()
                }
            }
        }
        #[cfg(not(target_os = "linux"))]
        "ERR ZEROOS/1 UNSUPPORTED_PLATFORM".into()
    }

    fn finish_recovery_mutation(&mut self, success: bool) {
        let mutation = self.recovery_mutation.take();
        #[cfg(target_os = "linux")]
        let _ = set_stdin_nonblocking(true);
        if !success || mount_data().is_err() {
            self.data = "failed";
            return;
        }
        self.data = "mounted";
        if matches!(mutation, Some(RecoveryMutation::FactoryReset)) {
            let remain_in_recovery = self.boot_state.booting == Some(Slot::Recovery);
            if self.boot_state.reset_trials().is_err() {
                self.data = "failed";
                return;
            }
            #[cfg(target_os = "linux")]
            if let Some(journal) = partition_device("ZEROOS-STATE") {
                match zeroos_storage::reconstruct_journal(&journal, &self.boot_state) {
                    Ok(state) => self.boot_state = state,
                    Err(_) => self.data = "failed",
                }
            }
            if remain_in_recovery {
                self.boot_state.booting = Some(Slot::Recovery);
            }
        }
        #[cfg(feature = "acceptance")]
        match mutation {
            Some(RecoveryMutation::RepairData) => accept("after-repair-data"),
            Some(RecoveryMutation::FactoryReset) => accept("after-factory-reset"),
            None => {}
        }
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
            [_, "UPDATE", "CHECK"] => self.update(false),
            [_, "UPDATE", "INSTALL"] => self.update(true),
            [_, "RECOVERY"] => {
                if self.recovery_busy() {
                    Err("BUSY")
                } else {
                    let mut requested = self.boot_state;
                    if requested.request_recovery().is_err() {
                        return "ERR ZEROOS/1 STATE_WRITE_FAILED".into();
                    }
                    #[cfg(all(target_os = "linux", not(test)))]
                    {
                        match partition_device("ZEROOS-STATE") {
                            Some(journal) => {
                                match zeroos_storage::write_journal(&journal, &requested) {
                                    Ok(()) => {
                                        self.boot_state = requested;
                                        Ok(())
                                    }
                                    Err(_) => Err("STATE_WRITE_FAILED"),
                                }
                            }
                            None => Err("NO_STATE"),
                        }
                    }
                    #[cfg(any(not(target_os = "linux"), test))]
                    {
                        self.boot_state = requested;
                        Ok(())
                    }
                }
            }
            [_, "SHUTDOWN"] => {
                if self.shutting_down {
                    Err("SHUTTING_DOWN")
                } else if self.update_pid.is_some() || self.recovery_pid.is_some() {
                    Err("BUSY")
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
            ["help"] => "help status logs start <service> stop <service> restart <service> update check|install reboot recovery repair-boot repair-data factory-reset ERASE-USER-DATA api-version selftest shutdown".into(),
            ["status"] => self.dispatch("ZEROOS/1 STATUS"),
            ["logs"] => {
                let response = self.dispatch("ZEROOS/1 LOGS");
                #[cfg(target_os = "linux")]
                if matches!(self.selftest, Some(Selftest::AwaitLogs)) {
                    self.log("INFO", "selftest", "logs-retrieved", "pass");
                    #[cfg(target_os = "linux")]
                    // SAFETY: PID 1 is the current process by Linux contract and signal 15 is a
                    // scalar value. No pointer, initialization, aliasing, alignment, lifetime, or
                    // shared-memory invariant is involved; the atomic handler is thread-safe and a
                    // failed notification leaves the existing shutdown state intact.
                    unsafe {
                        kill(1, 15);
                    }
                }
                response
            }
            ["start", name] => self.dispatch(&format!("ZEROOS/1 START {name}")),
            ["stop", name] => self.dispatch(&format!("ZEROOS/1 STOP {name}")),
            ["restart", name] => self.dispatch(&format!("ZEROOS/1 RESTART {name}")),
            ["update", "check"] => self.dispatch("ZEROOS/1 UPDATE CHECK"),
            ["update", "install"] => self.dispatch("ZEROOS/1 UPDATE INSTALL"),
            ["reboot", "recovery"] => self.dispatch("ZEROOS/1 RECOVERY"),
            ["repair-boot"] if self.boot_state.booting == Some(Slot::Recovery) => self.repair_boot(),
            ["repair-data"] if self.boot_state.booting == Some(Slot::Recovery) => self.repair_data(),
            ["factory-reset", confirmation] if self.boot_state.booting == Some(Slot::Recovery) => {
                self.factory_reset(confirmation)
            }
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
        let (Some(flaky), Some(dependent), Some(independent)) = (
            Self::index("flaky"),
            Self::index("dependent"),
            Self::index("independent"),
        ) else {
            self.log("ERROR", "selftest", "invalid-service-graph", "aborted");
            self.selftest = None;
            return;
        };
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
        // SAFETY: `sync` has no arguments or Rust memory access. It cannot violate provenance,
        // initialization, aliasing, alignment, lifetimes, or thread safety; it synchronizes kernel
        // filesystem state, and failure has no Rust cleanup obligation or owned resource to leak.
        unsafe {
            sync();
        }
        self.log("INFO", "core", "shutdown-complete", "state-synced=true");
    }

    #[cfg(target_os = "linux")]
    fn reap(&mut self) {
        loop {
            let mut status = 0;
            // SAFETY: `status` is initialized writable stack storage, uniquely borrowed for this
            // call and correctly aligned for `i32`; `waitpid` receives no retained pointer. PID -1
            // and WNOHANG=1 are valid Linux scalars. The call is serialized in PID 1's loop and a
            // failure leaves child ownership intact for a later reap, with no cleanup leak.
            let pid = unsafe { waitpid(-1, &mut status, 1) };
            if pid <= 0 {
                break;
            }
            self.child_exit(pid as u32, status == 0, Instant::now());
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

#[cfg(feature = "acceptance")]
fn accept(phase: &str) {
    println!("ZEROOS_ACCEPT phase={phase}");
    let _ = io::stdout().flush();
    thread::sleep(Duration::from_millis(250));
}

fn update_sequence(output: &str, install: bool) -> Option<u64> {
    if output.is_empty()
        || output.lines().any(|line| {
            line.len() > 512
                || !line.starts_with("ZEROOS_UPDATE ")
                || !line
                    .bytes()
                    .all(|byte| byte == b' ' || byte.is_ascii_graphic())
                || ["url=", "passphrase=", "recovery-code=", "private-key="]
                    .iter()
                    .any(|secret| line.contains(secret))
        })
    {
        return None;
    }
    output.lines().find_map(|line| {
        let fields: Vec<_> = line.split_whitespace().collect();
        let complete = fields.contains(&"phase=complete");
        let state = if install {
            "state=staged"
        } else {
            "state=available"
        };
        if !complete || !fields.contains(&state) {
            return None;
        }
        fields
            .iter()
            .find_map(|field| field.strip_prefix("sequence=")?.parse().ok())
    })
}

extern "C" fn on_sigchld(_: i32) {
    SIGCHLD_PENDING.store(true, Ordering::Relaxed);
}

extern "C" fn on_shutdown(_: i32) {
    SHUTDOWN_PENDING.store(true, Ordering::Relaxed);
}

#[cfg(target_os = "linux")]
fn install_signals() {
    // SAFETY: the installed handlers use the C ABI and only store to lock-free atomics. Function
    // pointers are static, initialized, aligned, and valid for process lifetime; no aliasing or
    // borrowed data is involved. Registration is single-threaded before worker creation and a
    // partial registration leaves each already-installed handler independently valid.
    unsafe {
        signal(17, on_sigchld as *const () as usize);
        signal(15, on_shutdown as *const () as usize);
        signal(2, on_shutdown as *const () as usize);
    }
}

#[cfg(target_os = "linux")]
fn mount_pseudo_filesystems() -> io::Result<()> {
    std::fs::create_dir_all("/proc")?;
    std::fs::create_dir_all("/sys")?;
    std::fs::create_dir_all("/dev")?;
    std::fs::create_dir_all("/run")?;
    for (source, target, filesystem) in [
        (c"proc", c"/proc", c"proc"),
        (c"sysfs", c"/sys", c"sysfs"),
        (c"devtmpfs", c"/dev", c"devtmpfs"),
        (c"tmpfs", c"/run", c"tmpfs"),
    ] {
        // SAFETY: all arguments are static NUL-terminated C strings or null, so provenance,
        // initialization, alignment, and lifetime hold for the call; the kernel does not retain
        // them. Mount setup is single-threaded, aliases no mutable Rust state, and failure is
        // reported without acquiring an owned resource that requires cleanup.
        if unsafe {
            mount(
                source.as_ptr(),
                target.as_ptr(),
                filesystem.as_ptr(),
                0,
                std::ptr::null(),
            )
        } != 0
            && io::Error::last_os_error().raw_os_error() != Some(16)
        {
            return Err(io::Error::last_os_error());
        }
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn start_data(runtime: &mut Runtime) -> Result<(), &'static str> {
    let device = partition_device("ZEROOS-DATA").ok_or("NO_DATA")?;
    let mut header = [0; 4096];
    fs::File::open(&device)
        .and_then(|mut file| file.read_exact(&mut header))
        .map_err(|_| "DATA_READ_FAILED")?;
    let command = if header.starts_with(b"LUKS\xba\xbe") {
        "unlock"
    } else if header.iter().all(|byte| *byte == 0) {
        "provision"
    } else {
        return Err("DATA_FORMAT_INVALID");
    };
    let status = std::process::Command::new("/zeroos-data")
        .arg(command)
        .arg(&device)
        .status()
        .map_err(|_| "DATA_ENGINE_FAILED")?;
    if !status.success() {
        return Err("DATA_ENGINE_FAILED");
    }
    mount_data()?;
    runtime.data = "mounted";
    Ok(())
}

#[cfg(target_os = "linux")]
fn mount_data() -> Result<(), &'static str> {
    fs::create_dir_all("/var/lib/zeroos").map_err(|_| "DATA_MOUNT_FAILED")?;
    // SAFETY: source, target, and filesystem are static initialized NUL-terminated strings with
    // valid provenance, alignment, and lifetime; the kernel retains no Rust reference. Mounting is
    // serialized during PID 1 startup before services, creates no alias, and failure leaves the
    // mapper owned by the external engine for explicit recovery without Rust resource leakage.
    if unsafe {
        mount(
            c"/dev/mapper/zeroos-data".as_ptr(),
            c"/var/lib/zeroos".as_ptr(),
            c"ext4".as_ptr(),
            2 | 4,
            std::ptr::null(),
        )
    } != 0
    {
        return Err("DATA_MOUNT_FAILED");
    }
    Ok(())
}

#[cfg(not(target_os = "linux"))]
fn mount_data() -> Result<(), &'static str> {
    Err("UNSUPPORTED_PLATFORM")
}

#[cfg(target_os = "linux")]
fn unmount_data() -> Result<(), ()> {
    // SAFETY: the path is a static initialized NUL-terminated string with valid provenance,
    // alignment, and lifetime; the kernel retains no pointer. PID 1 serializes recovery mutation,
    // so no owned service I/O races this unmount. Failure changes no Rust ownership and requires no
    // partial cleanup.
    if unsafe { libc::umount2(c"/var/lib/zeroos".as_ptr(), 0) } == 0 {
        Ok(())
    } else {
        Err(())
    }
}

#[cfg(target_os = "linux")]
fn set_stdin_nonblocking(nonblocking: bool) -> io::Result<()> {
    // SAFETY: STDIN_FILENO is a scalar descriptor owned by the process; F_GETFL reads flags only,
    // retains no pointer, and changes no ownership or memory alias. Failure leaves the descriptor
    // unchanged with no cleanup obligation.
    let flags = unsafe { libc::fcntl(libc::STDIN_FILENO, libc::F_GETFL) };
    if flags < 0 {
        return Err(io::Error::last_os_error());
    }
    let updated = if nonblocking {
        flags | libc::O_NONBLOCK
    } else {
        flags & !libc::O_NONBLOCK
    };
    // SAFETY: `updated` is the current descriptor flag word with only O_NONBLOCK changed. The call
    // retains no pointer, transfers no ownership, and PID 1 serializes it with recovery child
    // spawning. Failure leaves deterministic descriptor ownership and needs no extra cleanup.
    if unsafe { libc::fcntl(libc::STDIN_FILENO, libc::F_SETFL, updated) } == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

#[cfg(target_os = "linux")]
fn partition_device(label: &str) -> Option<std::path::PathBuf> {
    std::fs::read_dir("/sys/class/block")
        .ok()?
        .flatten()
        .find_map(|entry| {
            let uevent = std::fs::read_to_string(entry.path().join("uevent")).ok()?;
            let path = uevent
                .lines()
                .any(|line| line.strip_prefix("PARTNAME=") == Some(label))
                .then(|| std::path::Path::new("/dev").join(entry.file_name()))?;
            zeroos_storage::validate_partition_device(&path, label)
                .is_ok()
                .then_some(path)
        })
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
    // SAFETY: this fixture runs before it creates threads; after `fork`, both branches use only
    // async-signal-safe `_exit` except the child test delay. No pointer crosses the ABI, so
    // provenance, initialization, aliasing, alignment, and lifetime are inapplicable. No Rust
    // resource cleanup is relied upon, and the deliberate orphan is reaped by PID 1.
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
fn read_console_lines(buffer: &mut Vec<u8>) -> io::Result<Vec<String>> {
    let mut chunk = [0; 256];
    loop {
        match io::stdin().read(&mut chunk) {
            Ok(0) => break,
            Ok(count) => {
                if buffer
                    .len()
                    .checked_add(count)
                    .is_none_or(|size| size > MAX_REQUEST)
                {
                    buffer.clear();
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "recovery command too large",
                    ));
                }
                buffer.extend_from_slice(&chunk[..count]);
            }
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => break,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error),
        }
    }
    let mut lines = Vec::new();
    while let Some(end) = buffer.iter().position(|byte| *byte == b'\n') {
        let mut bytes: Vec<_> = buffer.drain(..=end).collect();
        while matches!(bytes.last(), Some(b'\n' | b'\r')) {
            bytes.pop();
        }
        lines.push(
            String::from_utf8(bytes).map_err(|_| {
                io::Error::new(io::ErrorKind::InvalidData, "invalid recovery input")
            })?,
        );
    }
    Ok(lines)
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
    mount_pseudo_filesystems()?;
    std::fs::create_dir_all("/run/zeroos")?;
    if std::path::Path::new(SOCKET).exists() {
        std::fs::remove_file(SOCKET)?;
    }
    let listener = std::os::unix::net::UnixListener::bind(SOCKET)?;
    std::fs::set_permissions(SOCKET, std::os::unix::fs::PermissionsExt::from_mode(0o600))?;
    listener.set_nonblocking(true)?;
    let mut runtime = Runtime::new();
    if let Some(journal) = partition_device("ZEROOS-STATE")
        && let Ok(state) = zeroos_storage::read_journal(&journal)
    {
        runtime.boot_state = state;
    }
    let data_started = start_data(&mut runtime);
    runtime.log(
        "INFO",
        "core",
        "api-ready",
        "version=1 socket=/run/zeroos/core-v1.sock mode=0600",
    );
    match data_started {
        Ok(()) => println!("zeroOS init: READY"),
        Err(error) => runtime.log("ERROR", "data", "startup-failed", error),
    }
    print!("zeroos recovery> ");
    io::stdout().flush()?;

    set_stdin_nonblocking(true)?;
    let mut console_input = Vec::with_capacity(MAX_REQUEST);

    // ponytail: polling is enough for M2's fixed service count; move signals and I/O to epoll/eventfd when runtime load warrants it.
    while !runtime.shutting_down {
        SIGCHLD_PENDING.store(false, Ordering::Relaxed);
        runtime.reap();
        if SHUTDOWN_PENDING.swap(false, Ordering::Relaxed) {
            runtime.shutting_down = true;
            break;
        }
        serve(&mut runtime, &listener);
        if runtime.recovery_mutation.is_none() {
            for line in read_console_lines(&mut console_input)? {
                let response = runtime.console(&line);
                if !response.is_empty() {
                    println!("{response}");
                }
                print!("zeroos recovery> ");
                io::stdout().flush()?;
            }
        }
        runtime.reset_healthy_budgets(Instant::now());
        runtime.confirm_health(Instant::now());
        runtime.advance_selftest();
        thread::sleep(Duration::from_millis(10));
    }
    runtime.shutdown();
    #[cfg(feature = "acceptance")]
    accept("before-reboot");
    // SAFETY: `reboot` receives only the valid Linux power-off scalar after all children are reaped
    // and filesystems synchronized. No pointer, initialization, aliasing, alignment, lifetime, or
    // shared-memory invariant is involved; failure returns to Rust with no resource transfer or
    // partial-cleanup obligation.
    unsafe {
        let command = if runtime.reboot_after_shutdown {
            RB_AUTOBOOT
        } else {
            RB_POWER_OFF
        };
        if reboot(command) == 0 {
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

    fn required_index(name: &str) -> Result<usize, Box<dyn std::error::Error>> {
        Runtime::index(name).ok_or_else(|| format!("missing test service {name}").into())
    }

    #[test]
    fn graph_and_orders_are_valid() {
        for (index, service) in SERVICES.iter().enumerate() {
            for dependency in service.deps {
                assert!(Runtime::index(dependency).is_some_and(|dependency| dependency < index));
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
    fn restart_window_exhausts_and_healthy_runtime_resets() -> Result<(), Box<dyn std::error::Error>>
    {
        let mut runtime = Runtime::new();
        let index = required_index("flaky")?;
        runtime.services[index].desired = true;
        let now = Instant::now();
        for pid in 1..=4 {
            runtime.services[index].pid = Some(pid);
            runtime.services[index].state = State::Running;
            runtime.child_exit(pid, false, now + Duration::from_millis(pid as u64));
        }
        assert_eq!(runtime.services[index].state, State::Failed);
        runtime.services[index].state = State::Running;
        runtime.services[index].started = Some(now);
        runtime.services[index].failures.push_back(now);
        runtime.reset_healthy_budgets(now + RESTART_WINDOW);
        assert!(runtime.services[index].failures.is_empty());
        Ok(())
    }

    #[test]
    fn dependency_failure_isolated() -> Result<(), Box<dyn std::error::Error>> {
        let mut runtime = Runtime::new();
        let flaky = required_index("flaky")?;
        let dependent = required_index("dependent")?;
        let independent = required_index("independent")?;
        runtime.services[dependent].desired = true;
        runtime.services[dependent].state = State::Running;
        runtime.services[dependent].pid = Some(20);
        runtime.services[independent].desired = true;
        runtime.services[independent].state = State::Running;
        runtime.services[independent].pid = Some(30);
        runtime.fail_dependents(flaky);
        assert!(!runtime.services[dependent].desired);
        assert_eq!(runtime.services[independent].state, State::Running);
        Ok(())
    }

    #[test]
    fn administrative_start_recovers_dependencies_and_restart_stops_consumers_first()
    -> Result<(), Box<dyn std::error::Error>> {
        let mut runtime = Runtime::new();
        let base = required_index("base")?;
        let flaky = required_index("flaky")?;
        let dependent = required_index("dependent")?;
        runtime.services[flaky].state = State::Failed;
        runtime.services[flaky].failures.push_back(Instant::now());
        runtime.start("dependent", true)?;
        assert!(runtime.services[base].desired);
        assert_eq!(runtime.services[flaky].state, State::Stopped);
        assert!(runtime.services[flaky].failures.is_empty());

        for (index, pid) in [(flaky, 20), (dependent, 30)] {
            runtime.services[index].state = State::Running;
            runtime.services[index].pid = Some(pid);
            runtime.services[index].desired = true;
        }
        runtime.restart("flaky")?;
        assert_eq!(runtime.services[dependent].state, State::Stopping);
        assert_eq!(runtime.services[flaky].state, State::Stopping);
        let stops: Vec<_> = runtime
            .logs
            .iter()
            .filter(|record| record.event == "stop-sent")
            .map(|record| record.component.as_str())
            .collect();
        assert_eq!(stops, ["dependent", "flaky"]);
        Ok(())
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
    fn protocol_rejects_versions_and_bad_limits_without_mutation()
    -> Result<(), Box<dyn std::error::Error>> {
        let mut runtime = Runtime::new();
        let before = runtime.status();
        assert!(
            runtime
                .dispatch("ZEROOS/2 START base")
                .contains("UNSUPPORTED_VERSION")
        );
        assert_eq!(runtime.status(), before);
        assert!(runtime.dispatch("ZEROOS/1 UNKNOWN").contains("BAD_REQUEST"));
        let base = required_index("base")?;
        runtime.services[base].pid = Some(42);
        assert_eq!(
            runtime.dispatch("ZEROOS/1 FIXTURE LOG base 42 INFO fixture-event hello world"),
            "OK ZEROOS/1"
        );
        assert!(runtime.log_text().contains("fixture-event\thello world"));
        let (mut sender, receiver) = UnixStream::pair()?;
        sender.write_all(&vec![b'x'; MAX_REQUEST + 1])?;
        drop(sender);
        assert!(read_request(&receiver).is_none());
        Ok(())
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
    fn trial_confirms_only_after_data_and_ten_healthy_seconds() {
        let mut runtime = Runtime::new();
        assert_eq!(runtime.boot_state.stage(Slot::B, 2), Ok(()));
        assert_eq!(runtime.boot_state.select(), Ok(Slot::B));
        let now = Instant::now();
        runtime.confirm_health(now + Duration::from_secs(20));
        assert_eq!(runtime.boot_state.confirmed, Slot::A);
        runtime.data = "mounted";
        runtime.confirm_health(now);
        runtime.confirm_health(now + Duration::from_secs(10));
        assert_eq!(runtime.boot_state.confirmed, Slot::B);
    }

    #[test]
    fn console_only_dispatches_declared_commands() {
        let mut runtime = Runtime::new();
        assert!(runtime.console("api-version").starts_with("ZEROOS/1"));
        let status = runtime.console("status");
        for field in [
            "mode=normal",
            "slot=a",
            "confirmed=a",
            "pending=none",
            "sequence=0",
            "update=idle",
            "data=locked",
        ] {
            assert!(status.contains(field));
        }
        assert_eq!(runtime.console("update check"), "OK ZEROOS/1");
        assert_eq!(runtime.console("update install"), "OK ZEROOS/1");
        assert_eq!(runtime.boot_state.pending, Some(Slot::B));
        assert_eq!(runtime.console("reboot recovery"), "OK ZEROOS/1");
        assert!(runtime.console("echo unsafe").contains("BAD_COMMAND"));
        assert!(runtime.console("status | shutdown").contains("BAD_COMMAND"));
        runtime.boot_state.booting = Some(Slot::Recovery);
        assert!(runtime.console("factory-reset").contains("BAD_COMMAND"));
        assert_eq!(runtime.console("repair-boot"), "OK ZEROOS/1");
        assert_eq!(runtime.boot_state.booting, Some(Slot::Recovery));
        assert!(
            runtime
                .console("factory-reset WRONG")
                .contains("CONFIRMATION_REQUIRED")
        );
    }

    #[test]
    fn update_progress_is_bounded_and_secret_free() {
        assert_eq!(
            update_sequence(
                "ZEROOS_UPDATE phase=download\nZEROOS_UPDATE phase=complete state=staged sequence=7 slot=/dev/vda3\n",
                true
            ),
            Some(7)
        );
        assert_eq!(
            update_sequence(
                "ZEROOS_UPDATE phase=complete state=available sequence=8\n",
                false
            ),
            Some(8)
        );
        assert_eq!(
            update_sequence(
                "ZEROOS_UPDATE phase=complete state=staged sequence=7 url=https://secret\n",
                true
            ),
            None
        );
        assert_eq!(update_sequence(&"x".repeat(513), true), None);
    }
}
