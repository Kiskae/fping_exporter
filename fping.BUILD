load("@rules_foreign_cc//tools/build_defs:configure.bzl", "configure_make")

filegroup(
    name = "src",
    srcs = glob(["**"]),
    visibility = ["//visibility:public"]
)

configure_make(
    name = "fping",
    lib_source = ":src",
    binaries = ["fping"],
    configure_in_place = True,
    out_bin_dir = "sbin",
)
