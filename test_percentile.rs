fn main() {
    let values = vec\![1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10.0];
    
    // Test different methods
    println\!("Testing different percentile methods for p=0.50:");
    
    // Method 1: Simple floor index
    let idx1 = (values.len() as f64 * 0.50).floor() as usize;
    let idx1 = idx1.min(values.len() - 1);
    println\!("Method 1 (floor): index={}, value={}", idx1, values[idx1]);
    
    // Method 2: Ceiling of (n-1)*p
    let idx2 = ((values.len() - 1) as f64 * 0.50).ceil() as usize;
    println\!("Method 2 (ceil): index={}, value={}", idx2, values[idx2]);
    
    // Method 3: Floor of (n-1)*p
    let idx3 = ((values.len() - 1) as f64 * 0.50).floor() as usize;
    println\!("Method 3 (floor of (n-1)*p): index={}, value={}", idx3, values[idx3]);
    
    // Test for p=0.90
    println\!("\nTesting for p=0.90:");
    let idx4 = ((values.len() - 1) as f64 * 0.90).floor() as usize;
    println\!("Floor of (n-1)*p: index={}, value={}", idx4, values[idx4]);
    
    let idx5 = ((values.len() - 1) as f64 * 0.90).ceil() as usize;
    println\!("Ceil of (n-1)*p: index={}, value={}", idx5, values[idx5]);
    
    // Test for p=1.00
    println\!("\nTesting for p=1.00:");
    let idx6 = ((values.len() - 1) as f64 * 1.00) as usize;
    println\!("(n-1)*p: index={}, value={}", idx6, values[idx6]);
}
