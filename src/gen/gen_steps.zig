const std = @import("std");
const LazyPath = std.Build.LazyPath;

pub const SourceItem = struct { name: []u8, api_export: bool };

pub fn nvim_gen_sources(
    b: *std.Build,
    nlua0: *std.Build.Step.Compile,
    api_headers: *std.ArrayList(LazyPath),
    versiondef_git: LazyPath,
    version_lua: LazyPath,
    precompile: bool,
) !struct { *std.Build.Step.WriteFile, LazyPath } {
    const gen_headers = b.addWriteFiles();

    {
        const gen_step = b.addRunArtifact(nlua0);
        gen_step.addFileArg(b.path("src/gen/gen_ex_cmds.lua"));
        _ = gen_header(b, gen_step, "ex_cmds_enum.generated.h", gen_headers);
        gen_step.addFileArg(b.path("src/nvim/ex_cmds.lua"));
    }

    {
        const gen_step = b.addRunArtifact(nlua0);
        gen_step.addFileArg(b.path("src/gen/gen_options.lua"));
        _ = gen_header(b, gen_step, "options_enum.generated.h", gen_headers);
        _ = gen_header(b, gen_step, "option_vars.generated.h", gen_headers);
        gen_step.addFileArg(b.path("src/nvim/options.lua"));

        const test_gen_step = b.step("wipopt", "debug one nlua0 (options)");
        test_gen_step.dependOn(&gen_step.step);
    }

    {
        const gen_step = b.addRunArtifact(nlua0);
        gen_step.addFileArg(b.path("src/gen/gen_events.lua"));
        _ = gen_header(b, gen_step, "auevents_enum.generated.h", gen_headers);
        gen_step.addFileArg(b.path("src/nvim/auevents.lua"));
    }

    _ = precompile;

    const ui_metadata = ui_step: {
        const gen_step = b.addRunArtifact(nlua0);
        gen_step.addFileArg(b.path("src/gen/gen_api_ui_events.lua"));
        gen_step.addFileArg(b.path("src/nvim/api/ui_events.in.h"));
        const ui_metadata = gen_step.addOutputFileArg("ui_metadata.mpack");
        break :ui_step ui_metadata;
    };

    const eval_funcs_metadata, const exported_funcs_metadata = dispatch_step: {
        const gen_step = b.addRunArtifact(nlua0);
        gen_step.addFileArg(b.path("src/gen/gen_api_dispatch.lua"));
        const exported_funcs_metadata = gen_step.addOutputFileArg("exported_funcs_metadata.mpack");
        const eval_funcs_metadata = gen_step.addOutputFileArg("eval_funcs_metadata.mpack");
        gen_step.addFileArg(b.path("src/nvim/api/dispatch_deprecated.lua"));
        // now follows all .h files with exported functions
        for (api_headers.items) |h| {
            gen_step.addFileArg(h);
        }

        break :dispatch_step .{ eval_funcs_metadata, exported_funcs_metadata };
    };

    {
        const gen_step = b.addRunArtifact(nlua0);
        gen_step.addFileArg(b.path("src/gen/gen_api_metadata.lua"));
        gen_step.addFileArg(exported_funcs_metadata);
        gen_step.addFileArg(ui_metadata);
        gen_step.addFileArg(b.path("src/nvim/api/ui.h"));
        gen_step.addFileArg(versiondef_git);
        gen_step.addFileArg(version_lua);
        gen_step.addFileArg(b.path("src/gen/dump_bin_array.lua"));
        _ = gen_header(b, gen_step, "api/private/api_metadata.generated.h", gen_headers);
    }

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
