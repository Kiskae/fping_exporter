workspace(name = "fping_exporter")

load("@bazel_tools//tools/build_defs/repo:http.bzl", "http_archive")

# configure/make compilation
http_archive(
   name = "rules_foreign_cc",
   strip_prefix = "rules_foreign_cc-master",
   url = "https://github.com/bazelbuild/rules_foreign_cc/archive/master.zip",
)

load("@rules_foreign_cc//:workspace_definitions.bzl", "rules_foreign_cc_dependencies")

rules_foreign_cc_dependencies()

# Fping release
http_archive(
   name = "fping",
   build_file = "@//:fping.BUILD",
   strip_prefix = "fping-5.0",
   urls = ["https://fping.org/dist/fping-5.0.tar.gz"],
)
