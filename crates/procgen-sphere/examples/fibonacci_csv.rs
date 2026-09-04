use procgen_sphere::{FibonacciConfig, fibonacci_sphere};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let count = std::env::args()
        .nth(1)
        .map(|value| value.parse())
        .transpose()?
        .unwrap_or(256);
    let mut config = FibonacciConfig::new(count);
    config.jitter = 0.5;
    config.seed = 7;

    println!("x,y,z");
    for point in fibonacci_sphere(config)? {
        println!("{},{},{}", point.x, point.y, point.z);
    }

    Ok(())
}
