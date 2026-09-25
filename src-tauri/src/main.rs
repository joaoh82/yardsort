// Prevents an additional console window on Windows in release. DO NOT REMOVE.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // Each of these can take over the process entirely, before Tauri is touched: this
    // executable doubles as the environment probe, as the terminal daemon, and as the hook an
    // agent runs to report what it is doing.
    yardsort_lib::print_env_and_exit_if_asked();
    yardsort_lib::run_daemon_and_exit_if_asked();
    yardsort_lib::run_hook_and_exit_if_asked();
    yardsort_lib::run()
}
