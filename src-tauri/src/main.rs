// Hide the console window on Windows release builds. No-op on macOS, kept so a
// future cross-platform build does not regress.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    mesh2motion_lib::run()
}
