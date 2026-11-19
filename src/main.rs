#![windows_subsystem = "windows"]

std::arch::global_asm!(include_str!("manifest.s"), options(raw));

mod sensors;
mod startup;
mod ui;
mod win;

fn main() {
    ui::run();
}
