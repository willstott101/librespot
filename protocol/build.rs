extern crate protobuf_codegen_pure;
extern crate regex;
extern crate crc;
use protobuf_codegen_pure::Customize;
use regex::Regex;
use std::path::Path;
use std::fs::File;
use std::io::prelude::*;
use crc::crc32::checksum_ieee;

fn read_to_string(path: String) -> Option<std::string::String> {
    let mut file = File::open(path).unwrap();
    let mut f_str = String::new();
    file.read_to_string(&mut f_str).unwrap();
    Some(f_str)
}

fn main() {
    let line_re = Regex::new(r"pub mod (?P<name>\w+);(?: ?/?/? ?(?P<CRC>\d+)?)?(?P<crlf>\r?\n)").unwrap();

    let mut lib_str = read_to_string("src/lib.rs".to_string()).unwrap();
    let mut changed = false;

    for cap in line_re.captures_iter(&lib_str.clone()) {
        let name = cap.name("name").unwrap().as_str();

        let dest = &format!("src/{}.rs", name);
        let mut regen = !Path::new(dest).exists();

        let path = &format!("proto/{}.proto", name);
        let contents = read_to_string(path.to_string()).unwrap().replace("\r\n", "\n");
        let new_crc = checksum_ieee(&contents.into_bytes()).to_string();
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
            let line_ending = cap.name("crlf").unwrap().as_str();
            lib_str = lib_str.replace(&cap[0], &format!("pub mod {}; // {}{}", name, new_crc, line_ending));
        } else {
            println!("CRC Matches for {}.proto not re-building.", name);
        }
    }

    if changed {
        // Write new checksums to file
        let mut file = File::create("src/lib.rs").unwrap();
        file.write_all(lib_str.as_bytes()).unwrap();
    }
}
