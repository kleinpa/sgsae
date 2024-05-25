package(default_visibility = ["//visibility:public"])

load("@rules_rust//rust:defs.bzl", "rust_binary", "rust_doc")

rust_binary(
    name = "cli",
    srcs = ["main.rs"],
    deps = ["//sgsae"],
)

rust_doc(
    name = "cli_doc",
    crate = ":cli",
)
