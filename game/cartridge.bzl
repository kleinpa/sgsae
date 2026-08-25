"""Rules for building guest code for the console.

The emulator, the CLI and the web bundle are built for the host. A cartridge is
not: it is RV32 bare-metal code. Rather than asking everyone to remember
`--platforms=//platforms:rv32` on the command line, `cartridge` carries an
outgoing transition, so a plain `bazel build //game:cartridge` configures its
dependency for the guest platform and leaves the rest of the build alone.
"""

load("@rules_rust//rust:defs.bzl", "rust_binary")

def _guest_transition_impl(_settings, _attr):
    return {"//command_line_option:platforms": "//platforms:rv32"}

_guest_transition = transition(
    implementation = _guest_transition_impl,
    inputs = [],
    outputs = ["//command_line_option:platforms"],
)

def _cartridge_impl(ctx):
    # Give the linked binary a name that says what it is. `ctx.actions.symlink`
    # is the built-in way to do this: no subprocess, and nothing to get wrong
    # across platforms the way a `cp` invocation would.
    out = ctx.actions.declare_file(ctx.label.name + ".elf")
    ctx.actions.symlink(
        output = out,
        target_file = ctx.executable.binary,
        progress_message = "Packaging cartridge %{label}",
    )

    return [DefaultInfo(files = depset([out]), runfiles = ctx.runfiles(files = [out]))]

cartridge = rule(
    implementation = _cartridge_impl,
    doc = "Builds a guest binary for //platforms:rv32 and names it .elf.",
    attrs = {
        "binary": attr.label(
            doc = "The guest binary. Built for the console, not the host.",
            cfg = _guest_transition,
            executable = True,
            mandatory = True,
        ),
        "_allowlist_function_transition": attr.label(
            default = "@bazel_tools//tools/allowlists/function_transition_allowlist",
        ),
    },
)

def cartridge_binary(name, srcs, deps = [], **kwargs):
    """Build one cartridge: a guest binary, packaged as `<name>.elf`.

    Wraps the flags every cartridge needs. Notably `-Ctarget-feature=-c` and
    `--no-relax` are both there because the only bare-metal triple rules_rust
    offers is `riscv32imc`, and the machine implements RV32C rather than
    fighting that; keeping the flags means a cartridge stays readable in a
    disassembler.
    """
    rust_binary(
        name = name + "_bin",
        srcs = srcs,
        compile_data = ["//sdk:console.ld"],
        edition = "2024",
        rustc_flags = [
            "-Clink-arg=-T$(location //sdk:console.ld)",
            "-Clink-arg=--no-relax",
            "-Ctarget-feature=-c",
            # A cartridge is always optimised, whatever -c the host build uses.
            "-Copt-level=2",
            "-Cpanic=abort",
        ],
        target_compatible_with = [
            "@platforms//cpu:riscv32",
            "@platforms//os:none",
        ],
        deps = ["//sdk:console"] + deps,
        **kwargs
    )
    cartridge(
        name = name,
        binary = ":" + name + "_bin",
    )
