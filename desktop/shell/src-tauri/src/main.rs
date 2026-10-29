// no console window on Windows release
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    life_desktop_lib::run()
}
