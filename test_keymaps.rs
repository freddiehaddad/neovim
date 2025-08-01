// Test file for verifying keymap functionality
fn main() {
    println!("Testing various keymaps:");
    println!("- Normal mode: h,j,k,l for movement");
    println!("- Insert mode: i to enter, Escape to exit");
    println!("- Visual mode: v to enter, d to delete selection");
    println!("- Command mode: : to enter, :q to quit");
    println!("- Search: / to search forward, n for next");

    let test_data = vec![1, 2, 3, 4, 5];
    for item in test_data {
        println!("Item: {}", item);
    }

    // Test some code structures
    if true {
        println!("Conditional code");
    }

    let result = match 42 {
        42 => "The answer",
        _ => "Something else",
    };

    println!("Result: {}", result);
}
