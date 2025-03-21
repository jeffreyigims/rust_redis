# Rust Redis 

The repo includes a key-value store server written in rust. 

The goal of the project is to build a key-value storage service that's written in rust and included minimal external dependencies. I.e. I implement my own version of an asynchronous input/output using a non-blocking custom-build event loop that only depends on `libc` and `std::net`.
