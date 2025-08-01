use tree_sitter::{Language, Parser};

fn main() {
    let language: Language = tree_sitter_rust::LANGUAGE.into();
    let mut parser = Parser::new();
    parser.set_language(&language).unwrap();

    let test_code = "fn test() { let x = 5; }";

    if let Some(tree) = parser.parse(test_code, None) {
        let root_node = tree.root_node();
        print_tree(&root_node, test_code, 0);
    }
}

fn print_tree(node: &tree_sitter::Node, source: &str, depth: usize) {
    let indent = "  ".repeat(depth);
    let node_text = &source[node.start_byte()..node.end_byte()];
    println!("{}kind: '{}', text: '{}'", indent, node.kind(), node_text);

    for i in 0..node.child_count() {
        if let Some(child) = node.child(i) {
            print_tree(&child, source, depth + 1);
        }
    }
}
