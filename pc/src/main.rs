#![windows_subsystem = "windows"]

mod capture;
mod devtools;
mod dxgi;
mod log;
mod monitors;
mod net;
mod pen;
mod settings;
mod tray;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    // --monitor N: выбрать монитор только на этот запуск (для проверок)
    let from_arg = args.iter().position(|a| a == "--monitor").and_then(|i| args.get(i + 1)?.parse().ok());
    monitors::select(from_arg.or(settings::load().monitor));

    if devtools::run(&args) {
        return;
    }
    if !tray::single_instance() {
        return;
    }
    log::init();
    log!("запуск, монитор {:?}", monitors::selected());
    std::thread::spawn(net::serve_udp);
    std::thread::spawn(net::serve_screen);
    std::thread::spawn(net::serve_discovery);
    tray::run();
}
