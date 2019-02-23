extern crate protobuf;
// This file is parsed by build.rs
// Each included module will be compiled from the matching .proto definition.
// A CRC of the used proto file will be added as a comment.
pub mod authentication; // 284352592
pub mod keyexchange; // 1910242467
pub mod mercury; // 3661728422
pub mod metadata; // 3464762495
pub mod pubsub; // 2935982216
pub mod spirc; // 2909797125
