import { spawnSync } from "node:child_process";
import { mkdirSync } from "node:fs";
import { join } from "node:path";

// `fa-window.exe`, the borderless-window and cursor helper the client runs
// beside Forged Alliance under Wine on macOS (natives-src/fa-window/main.rs).
//
// Built rather than downloaded: it is this repository's own code, and it
// needs nothing but `rustc` and the Windows target's core library. No C
// compiler, no MinGW, no Windows SDK: it has no standard library and imports
// straight from user32 and kernel32, and `rust-lld` links it.

if (process.platform !== "darwin" || process.env.FAF_SKIP_FA_WINDOW_BUILD === "1") {
  process.exit(0);
}

const target = "x86_64-pc-windows-msvc";
const run = (command, args) => {
  const result = spawnSync(command, args, { stdio: "inherit" });
  if (result.status !== 0) {
    throw new Error(`${command} ${args.join(" ")} failed`);
  }
};

try {
  run("rustup", ["target", "add", target]);
  const out = join(process.cwd(), "natives");
  mkdirSync(out, { recursive: true });
  run("rustc", [
    "--edition", "2024",
    "--target", target,
    "-C", "panic=abort",
    "-C", "opt-level=s",
    "-C", "lto=fat",
    "-C", "codegen-units=1",
    "-C", "linker=rust-lld",
    "-C", "link-arg=/ENTRY:mainCRTStartup",
    "-C", "link-arg=/SUBSYSTEM:WINDOWS",
    "-C", "link-arg=/NODEFAULTLIB",
    join(process.cwd(), "natives-src", "fa-window", "main.rs"),
    "-o", join(out, "fa-window.exe"),
  ]);
  console.log("[fa-window] Built natives/fa-window.exe");
} catch (error) {
  console.error(`[fa-window] ${error.message}`);
  process.exitCode = 1;
}
