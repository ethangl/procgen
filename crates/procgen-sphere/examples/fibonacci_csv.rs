use procgen_sphere::{FibonacciConfig, fibonacci_sphere};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let count = std::env::args()
        .nth(1)
        .map(|value| value.parse())
        .transpose()?
        .unwrap_or(256);
    let config = FibonacciConfig {
        jitter: 0.5,
        seed: 7,
        ..FibonacciConfig::new(count)
    };

    println!("x,y,z");
    for point in fibonacci_sphere(config)? {
        println!("{},{},{}", point.x, point.y, point.z);
    }

    Ok(())
}
