fn main() {
    println!("async magenta...");
    legion_core::keyboard::set_rgb_static_async(255, 0, 128);
    std::thread::sleep(std::time::Duration::from_millis(1000));
    println!("bri={:?}", legion_core::keyboard::rgb_brightness());
    println!("peek={:?}", legion_core::keyboard::peek_effect_rgb());
    println!("sync cyan...");
    match legion_core::keyboard::set_rgb_static(0, 200, 255) {
        Ok(()) => println!("sync ok"),
        Err(e) => println!("sync err {e}"),
    }
    println!("peek={:?}", legion_core::keyboard::peek_effect_rgb());
}
