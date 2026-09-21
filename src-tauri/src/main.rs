// Prevents an additional console window on Windows in release. DO NOT REMOVE.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // Both of these can take over the process entirely, before Tauri is touched: this executable
    // doubles as the environment probe and as the terminal daemon.
    yardsort_lib::print_env_and_exit_if_asked();
    yardsort_lib::run_daemon_and_exit_if_asked();
    yardsort_lib::run()
}
