use std::{io::Write, thread};

const RB_POWER_OFF: i32 = 0x4321_fedc;

unsafe extern "C" {
    fn sync();
    fn reboot(command: i32) -> i32;
}

fn main() {
    println!("zeroOS init: READY");
    std::io::stdout().flush().expect("flush readiness line");
    unsafe {
        sync();
        if reboot(RB_POWER_OFF) == 0 {
            return;
        }
    }
    eprintln!(
        "zeroOS init: poweroff failed: {}",
        std::io::Error::last_os_error()
    );
    loop {
        thread::park();
    }
}
