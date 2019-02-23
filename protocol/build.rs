extern crate protobuf_codegen_pure;
extern crate regex;
extern crate crc;
use protobuf_codegen_pure::Customize;
use regex::Regex;
use std::path::Path;
use std::fs::{read, read_to_string, write};
use crc::crc32::checksum_ieee;

fn main() {
    let line_re = Regex::new(r"pub mod (?P<name>\w+);(?: ?/?/? ?(?P<CRC>\d+)?)?\n?").unwrap();

    let mut lib_str = read_to_string("src/lib.rs").unwrap();
    let mut changed = false;

    for cap in line_re.captures_iter(&lib_str.clone()) {
        let name = cap.name("name").unwrap().as_str();

        let dest = &format!("src/{}.rs", name);
        let mut regen = !Path::new(dest).exists();

        let path = &format!("proto/{}.proto", name);
        let new_crc = checksum_ieee(&read(path).unwrap()).to_string();
        if !regen {
            match cap.name("CRC") {
                Some(crc) => regen = crc.as_str() != new_crc,
                None      => regen = true,
            }
        }

        if regen {
            println!("Regenerating {} from {}", dest, path);
            protobuf_codegen_pure::run(protobuf_codegen_pure::Args {
                out_dir: "src",
                input: &[path],
                includes: &["proto"],
                customize: Customize { ..Default::default() },
            }).expect("protoc");

            changed = true;
            lib_str = lib_str.replace(&cap[0], &format!("pub mod {}; // {}\n", name, new_crc));
        } else {
            println!("CRC Matches for {}.proto not re-building.", name);
        }
    }

    if changed {
        // Write new checksums to file
        write("src/lib.rs", lib_str).unwrap();
    }
}
