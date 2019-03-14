extern crate protobuf_codegen; // Does the business
extern crate protobuf_codegen_pure; // Helper function
extern crate regex;

use std::path::Path;
use std::fs::{read_to_string, write};

use protobuf_codegen_pure::Customize;
use protobuf_codegen_pure::parse_and_typecheck;

use regex::Regex;

fn main() {
    let customizations = Customize { ..Default::default() };

    let line_re = Regex::new(r"pub mod (\w+);").unwrap();
    let lib_str = read_to_string("src/lib.rs").unwrap();

    // Iterate over the desired module names.
    for cap in line_re.captures_iter(&lib_str) {
        let name = cap.get(1).unwrap().as_str();

        // Build the paths to relevant files.
        let src = &format!("proto/{}.proto", name);
        let dest = &format!("src/{}.rs", name);

        // Get the contents of the existing generated file.
        let mut existing = "".to_string();
        if Path::new(dest).exists() {
            // Removing CRLF line endings if present.
            existing = read_to_string(dest).unwrap().replace("\r\n", "\n");
        }

        println!("Regenerating {} from {}", dest, src);

        // Parse the proto files as the protobuf-codegen-pure crate does.
        let p = parse_and_typecheck(&["proto"], &[src]).expect("protoc");
        // But generate them with the protobuf-codegen crate directly.
        // Then we can keep the result in-memory.
        let result = protobuf_codegen::gen(
            &p.file_descriptors,
            &p.relative_paths,
            &customizations,
        );
        // Protoc result as a byte array.
        let new = &result.first().unwrap().content;
        // Convert to utf8 to compare with existing.
        let new = std::str::from_utf8(&new).unwrap();
        // Save newly generated file if changed.
        if new != existing {
            write(dest, &new).unwrap();
        }
    }
}
