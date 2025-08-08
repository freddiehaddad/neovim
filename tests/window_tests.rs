use oxidized::core::window::{SplitDirection, Window, WindowManager};

#[test]
fn test_window_creation() {
    let window = Window::new(1, 10, 20, 80, 24);
    assert_eq!(window.id, 1);
    assert_eq!(window.x, 10);
    assert_eq!(window.y, 20);
    assert_eq!(window.width, 80);
    assert_eq!(window.height, 24);
    assert_eq!(window.viewport_top, 0);
    assert_eq!(window.cursor_row, 0);
    assert_eq!(window.cursor_col, 0);
    assert_eq!(window.buffer_id, None);
}

#[test]
fn test_window_buffer_assignment() {
    let mut window = Window::new(1, 0, 0, 80, 24);
    assert_eq!(window.buffer_id, None);

    window.set_buffer(42);
    assert_eq!(window.buffer_id, Some(42));
}

#[test]
fn test_window_cursor_position() {
    let mut window = Window::new(1, 0, 0, 80, 24);
    assert_eq!(window.get_cursor_position(), (0, 0));

    window.save_cursor_position(15, 30);
    assert_eq!(window.get_cursor_position(), (15, 30));
    assert_eq!(window.cursor_row, 15);
    assert_eq!(window.cursor_col, 30);
}

#[test]
fn test_window_content_height() {
    let window = Window::new(1, 0, 0, 80, 24);
    // Should reserve 1 line for status bar
    assert_eq!(window.content_height(), 23);

    let small_window = Window::new(2, 0, 0, 80, 1);
    // Should handle saturating subtraction
    assert_eq!(small_window.content_height(), 0);
}

#[test]
fn test_window_point_inside() {
    let window = Window::new(1, 10, 5, 20, 15);

    // Points inside the window
    assert!(window.is_point_inside(10, 5)); // Top-left corner
    assert!(window.is_point_inside(15, 10)); // Middle
    assert!(window.is_point_inside(29, 19)); // Bottom-right corner (exclusive)

    // Points outside the window
    assert!(!window.is_point_inside(9, 10)); // Left of window
    assert!(!window.is_point_inside(30, 10)); // Right of window
    assert!(!window.is_point_inside(15, 4)); // Above window
    assert!(!window.is_point_inside(15, 20)); // Below window
}

#[test]
fn test_window_manager_creation() {
    let manager = WindowManager::new(80, 24);

    // Should have one initial window
    assert_eq!(manager.all_windows().len(), 1);
    assert_eq!(manager.current_window_id(), Some(1));

    let current_window = manager.current_window().unwrap();
    assert_eq!(current_window.id, 1);
    assert_eq!(current_window.x, 0);
    assert_eq!(current_window.y, 0);
    assert_eq!(current_window.width, 80);
    assert_eq!(current_window.height, 22); // 24 - 2 for status lines
}

#[test]
fn test_window_manager_current_window_operations() {
    let mut manager = WindowManager::new(80, 24);

    // Test getting current window
    assert!(manager.current_window().is_some());

    // Test getting mutable current window
    assert!(manager.current_window_mut().is_some());

    // Test current window ID
    assert_eq!(manager.current_window_id(), Some(1));
}

#[test]
fn test_window_manager_set_current_window() {
    let mut manager = WindowManager::new(80, 24);

    // Setting to existing window should work
    assert!(manager.set_current_window(1));
    assert_eq!(manager.current_window_id(), Some(1));

    // Setting to non-existing window should fail
    assert!(!manager.set_current_window(999));
    assert_eq!(manager.current_window_id(), Some(1)); // Should remain unchanged
}

#[test]
fn test_window_manager_get_window() {
    let manager = WindowManager::new(80, 24);

    // Should be able to get existing window
    assert!(manager.get_window(1).is_some());

    // Should return None for non-existing window
    assert!(manager.get_window(999).is_none());
}

#[test]
fn test_window_manager_get_window_mut() {
    let mut manager = WindowManager::new(80, 24);

    // Should be able to get mutable reference to existing window
    assert!(manager.get_window_mut(1).is_some());

    // Should return None for non-existing window
    assert!(manager.get_window_mut(999).is_none());
}

#[test]
fn test_horizontal_split_default() {
    let mut manager = WindowManager::new(80, 24);

    // Split the initial window horizontally
    let new_window_id = manager.split_current_window(SplitDirection::Horizontal);
    assert!(new_window_id.is_some());
    let new_id = new_window_id.unwrap();

    // Should now have 2 windows
    assert_eq!(manager.all_windows().len(), 2);

    // Check original window (top)
    let original_window = manager.get_window(1).unwrap();
    assert_eq!(original_window.x, 0);
    assert_eq!(original_window.y, 0);
    assert_eq!(original_window.width, 80);
    assert_eq!(original_window.height, 11); // Half of 22 (content height)

    // Check new window (bottom)
    let new_window = manager.get_window(new_id).unwrap();
    assert_eq!(new_window.x, 0);
    assert_eq!(new_window.y, 11);
    assert_eq!(new_window.width, 80);
    assert_eq!(new_window.height, 11);
}

#[test]
fn test_horizontal_split_above() {
    let mut manager = WindowManager::new(80, 24);

    let new_window_id = manager.split_current_window(SplitDirection::HorizontalAbove);
    assert!(new_window_id.is_some());
    let new_id = new_window_id.unwrap();

    assert_eq!(manager.all_windows().len(), 2);

    // Check new window (top)
    let new_window = manager.get_window(new_id).unwrap();
    assert_eq!(new_window.x, 0);
    assert_eq!(new_window.y, 0);
    assert_eq!(new_window.height, 11);

    // Check original window (bottom)
    let original_window = manager.get_window(1).unwrap();
    assert_eq!(original_window.y, 11);
    assert_eq!(original_window.height, 11);
}

#[test]
fn test_vertical_split_default() {
    let mut manager = WindowManager::new(80, 24);

    let new_window_id = manager.split_current_window(SplitDirection::Vertical);
    assert!(new_window_id.is_some());
    let new_id = new_window_id.unwrap();

    assert_eq!(manager.all_windows().len(), 2);

    // Check original window (left)
    let original_window = manager.get_window(1).unwrap();
    assert_eq!(original_window.x, 0);
    assert_eq!(original_window.y, 0);
    assert_eq!(original_window.width, 40); // Half of 80
    assert_eq!(original_window.height, 22);

    // Check new window (right)
    let new_window = manager.get_window(new_id).unwrap();
    assert_eq!(new_window.x, 40);
    assert_eq!(new_window.y, 0);
    assert_eq!(new_window.width, 40);
    assert_eq!(new_window.height, 22);
}

#[test]
fn test_vertical_split_left() {
    let mut manager = WindowManager::new(80, 24);

    let new_window_id = manager.split_current_window(SplitDirection::VerticalLeft);
    assert!(new_window_id.is_some());
    let new_id = new_window_id.unwrap();

    assert_eq!(manager.all_windows().len(), 2);

    // Check new window (left)
    let new_window = manager.get_window(new_id).unwrap();
    assert_eq!(new_window.x, 0);
    assert_eq!(new_window.width, 40);

    // Check original window (right)
    let original_window = manager.get_window(1).unwrap();
    assert_eq!(original_window.x, 40);
    assert_eq!(original_window.width, 40);
}

#[test]
fn test_close_window_single_window() {
    let mut manager = WindowManager::new(80, 24);

    // Should not be able to close the last window
    assert!(!manager.close_current_window());
    assert_eq!(manager.all_windows().len(), 1);
    assert_eq!(manager.current_window_id(), Some(1));
}

#[test]
fn test_close_window_multiple_windows() {
    let mut manager = WindowManager::new(80, 24);

    // Create a split to have multiple windows
    let new_window_id = manager
        .split_current_window(SplitDirection::Horizontal)
        .unwrap();
    assert_eq!(manager.all_windows().len(), 2);

    // Close current window
    assert!(manager.close_current_window());
    assert_eq!(manager.all_windows().len(), 1);

    // Should have switched to the remaining window
    assert!(manager.current_window_id().is_some());
    let remaining_id = manager.current_window_id().unwrap();
    assert!(remaining_id == 1 || remaining_id == new_window_id);
}

#[test]
fn test_window_navigation() {
    let mut manager = WindowManager::new(160, 50);

    // Create a 2x2 grid of windows
    let right_window = manager
        .split_current_window(SplitDirection::Vertical)
        .unwrap();
    manager.set_current_window(1); // Focus left window
    let bottom_left = manager
        .split_current_window(SplitDirection::Horizontal)
        .unwrap();
    manager.set_current_window(right_window); // Focus right window
    let _bottom_right = manager
        .split_current_window(SplitDirection::Horizontal)
        .unwrap();

    // Start from top-left (window 1)
    manager.set_current_window(1);
    assert_eq!(manager.current_window_id(), Some(1));

    // Test navigation
    // Move right should work
    assert!(manager.move_to_window_right());
    assert_eq!(manager.current_window_id(), Some(right_window));

    // Move left should work
    assert!(manager.move_to_window_left());
    assert_eq!(manager.current_window_id(), Some(1));

    // Move down should work
    assert!(manager.move_to_window_down());
    assert_eq!(manager.current_window_id(), Some(bottom_left));

    // Move up should work
    assert!(manager.move_to_window_up());
    assert_eq!(manager.current_window_id(), Some(1));
}

#[test]
fn test_window_resizing() {
    let mut manager = WindowManager::new(80, 24);

    // Create a vertical split
    let right_window = manager
        .split_current_window(SplitDirection::Vertical)
        .unwrap();

    // Get original dimensions
    let left_window = manager.get_window(1).unwrap();
    let original_left_width = left_window.width;
    let right_window_obj = manager.get_window(right_window).unwrap();
    let _original_right_width = right_window_obj.width;

    // Test resizing wider
    assert!(manager.resize_current_window_wider(5));

    let left_after_resize = manager.get_window(1).unwrap();
    assert_eq!(left_after_resize.width, original_left_width + 5);

    // Test resizing narrower
    assert!(manager.resize_current_window_narrower(3));

    let left_after_narrow = manager.get_window(1).unwrap();
    assert_eq!(left_after_narrow.width, original_left_width + 5 - 3);
}

#[test]
fn test_window_buffer_preservation_on_split() {
    let mut manager = WindowManager::new(80, 24);

    // Set buffer for initial window
    {
        let initial_window = manager.get_window_mut(1).unwrap();
        initial_window.set_buffer(42);
        initial_window.save_cursor_position(10, 5);
    } // Release mutable borrow

    // Split the window
    let new_window_id = manager
        .split_current_window(SplitDirection::Horizontal)
        .unwrap();

    // Both windows should have the same buffer
    let original_window = manager.get_window(1).unwrap();
    let new_window = manager.get_window(new_window_id).unwrap();

    assert_eq!(original_window.buffer_id, Some(42));
    assert_eq!(new_window.buffer_id, Some(42));

    // Original window should have its cursor position reset
    // (Window constructor likely initializes cursor to (0, 0))
    assert_eq!(original_window.get_cursor_position(), (0, 0));
}

#[test]
fn test_split_direction_equality() {
    assert_eq!(SplitDirection::Horizontal, SplitDirection::Horizontal);
    assert_eq!(SplitDirection::Vertical, SplitDirection::Vertical);
    assert_ne!(SplitDirection::Horizontal, SplitDirection::Vertical);
    assert_ne!(
        SplitDirection::HorizontalAbove,
        SplitDirection::HorizontalBelow
    );
    assert_ne!(SplitDirection::VerticalLeft, SplitDirection::VerticalRight);
}
