#![windows_subsystem = "windows"]

std::arch::global_asm!(include_str!("manifest.s"), options(raw));

mod gdiplus;
mod island;
mod island_data;
mod liquid_glass;
mod lyrics;
mod media;
mod plugin;
mod sensors;
mod settings;
mod settings_ui;
mod shelf;
mod startup;
mod ui;
mod win;

fn main() {
    ui::run();
}
