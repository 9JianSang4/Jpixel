// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // Belt-and-suspenders: detach from any console that Windows may have
    // incorrectly assigned (e.g. when launched from Run key at boot on
    // certain Windows builds). Without this, closing the rogue console
    // window kills the process.
    #[cfg(target_os = "windows")]
    {
        extern "system" {
            fn FreeConsole() -> i32;
        }
        unsafe { FreeConsole(); }
    }
    jpixel_lib::run()
}
