const std = @import("std");
const LazyPath = std.Build.LazyPath;

pub const SourceItem = struct { name: []u8, api_export: bool };

pub fn nvim_gen_sources(
    b: *std.Build,
    nlua0: *std.Build.Step.Compile,
    api_headers: *std.ArrayList(LazyPath),
    precompile: bool,
) !struct { *std.Build.Step.WriteFile, LazyPath } {
    const gen_headers = b.addWriteFiles();

    _ = precompile;

    const eval_funcs_metadata = dispatch_step: {
        const gen_step = b.addRunArtifact(nlua0);
        gen_step.addFileArg(b.path("src/gen/gen_api_dispatch.lua"));
        const eval_funcs_metadata = gen_step.addOutputFileArg("eval_funcs_metadata.mpack");
        gen_step.addFileArg(b.path("src/nvim/api/dispatch_deprecated.lua"));
        // now follows all .h files with exported functions
        for (api_headers.items) |h| {
            gen_step.addFileArg(h);
        }

        break :dispatch_step eval_funcs_metadata;
    };

    const funcs_data = eval_step: {
        const gen_step = b.addRunArtifact(nlua0);
        gen_step.addFileArg(b.path("src/gen/gen_eval.lua"));
        gen_step.addFileArg(eval_funcs_metadata);
        const funcs_data = gen_step.addOutputFileArg("funcs_data.mpack");
        gen_step.addFileArg(b.path("src/nvim/eval.lua"));
        break :eval_step funcs_data;
    };

    return .{ gen_headers, funcs_data };
}

fn gen_header(
    b: *std.Build,
    gen_step: *std.Build.Step.Run,
    name: []const u8,
    gen_headers: *std.Build.Step.WriteFile,
) std.Build.LazyPath {
    _ = b;
    const header = gen_step.addOutputFileArg(name);
    _ = gen_headers.addCopyFile(header, name);
    return header;
}

pub const PreprocessorOptions = struct {
    include_dirs: []const LazyPath = &.{},
    c_macros: []const []const u8 = &.{},
    target: ?std.Build.ResolvedTarget = null,
};
