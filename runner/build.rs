fn main() {
    cc::Build::new()
        .cpp(true)
        .include("bocfel/include")
        .include("bocfel/src")
        .file("bocfel/src/bocfel_embed.cpp")
        .flag_if_supported("-std=c++17")
        .flag_if_supported("/std:c++17")
        .compile("bocfel_embedded");

    println!("cargo:rerun-if-changed=bocfel/src/bocfel_embed.cpp");
    println!("cargo:rerun-if-changed=bocfel/include/bocfel_embed.h");
}
