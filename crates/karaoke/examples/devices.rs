use karaoke::device::{
    cpu_threads_for_percent, describe, list_adapters, pick_gpu, DEFAULT_CPU_PERCENT,
    MIN_DISCRETE_VRAM_MB,
};

fn main() {
    let adapters = list_adapters();
    for a in &adapters {
        println!(
            "index: {}, name: {}, kind: {:?}, dedicated MB: {}, shared MB: {}",
            a.index, a.name, a.kind, a.dedicated_vram_mb, a.shared_system_mb
        );
    }

    let picked = pick_gpu(&adapters, MIN_DISCRETE_VRAM_MB);
    match picked {
        Some(a) => println!("{}", describe(a)),
        None => println!("none, CPU will be used"),
    }

    println!("cpu threads at 35%: {}", cpu_threads_for_percent(DEFAULT_CPU_PERCENT));
}
