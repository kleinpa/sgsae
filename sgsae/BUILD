load("@rules_rust//rust:defs.bzl", "rust_doc", "rust_library", "rust_test")

rust_library(
    name = "sgsae",
    srcs = [
        "instructions.rs",
        "lib.rs",
        "machine.rs",
        "types.rs",
    ],
    crate_name = "sgsae",
    visibility = ["//visibility:public"],
)

rust_test(
    name = "sgsae_test",
    crate = ":sgsae",
)

rust_doc(
    name = "sgsae_doc",
    crate = ":sgsae",
)
