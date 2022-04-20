package(default_visibility = ["//visibility:public"])

load("@rules_rust//rust:defs.bzl", "rust_doc", "rust_library")

rust_library(
    name = "sgsae",
    srcs = [
        "sgsae.rs",
    ],
)

rust_doc(
    name = "sgsae_doc",
    crate = ":sgsae",
)
