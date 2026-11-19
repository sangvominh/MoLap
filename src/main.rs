#![windows_subsystem = "windows"]

std::arch::global_asm!(include_str!("manifest.s"), options(raw));

mod sensors;
mod ui;
mod win;

fn main() {
    ui::run();
}
