#!/usr/bin/env node
// Monta os pacotes npm do CLI a partir dos binários pré-compilados por plataforma.
//
//   node scripts/build-npm.mjs --version 0.1.14 --binaries <dir>
//
// <dir> deve conter, por plataforma, `<rust-target>/claudinio[.exe]`.
// Saída em `npm/dist/`: o launcher `claudinio` + um `cli-<os>-<cpu>` por
// binário encontrado, prontos para `npm publish`.

import {
  existsSync,
  mkdirSync,
  cpSync,
  writeFileSync,
  readFileSync,
  chmodSync,
  rmSync,
} from "node:fs";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");

// Alinhado com a matriz do .github/workflows/release.yml.
const TARGETS = [
  { rust: "aarch64-apple-darwin", os: "darwin", cpu: "arm64" },
  { rust: "x86_64-unknown-linux-gnu", os: "linux", cpu: "x64" },
  { rust: "aarch64-unknown-linux-gnu", os: "linux", cpu: "arm64" },
  { rust: "x86_64-pc-windows-msvc", os: "win32", cpu: "x64" },
  { rust: "aarch64-pc-windows-msvc", os: "win32", cpu: "arm64" },
];

function arg(name, fallback) {
  const i = process.argv.indexOf(`--${name}`);
  return i >= 0 ? process.argv[i + 1] : fallback;
}

const version =
  arg("version") ||
  JSON.parse(readFileSync(join(root, "npm/claudinio/package.json"), "utf8")).version;
const binDir = arg("binaries");
if (!binDir) {
  console.error("uso: build-npm.mjs --version <v> --binaries <dir>");
  process.exit(1);
}

const distRoot = join(root, "npm/dist");
rmSync(distRoot, { recursive: true, force: true });
mkdirSync(distRoot, { recursive: true });

const built = [];
for (const t of TARGETS) {
  const binName = t.os === "win32" ? "claudinio.exe" : "claudinio";
  const src = join(binDir, t.rust, binName);
  if (!existsSync(src)) {
    console.warn(`(pulando ${t.os}-${t.cpu}: ${src} ausente)`);
    continue;
  }
  const pkgName = `cli-${t.os}-${t.cpu}`;
  const pkgDir = join(distRoot, pkgName);
  mkdirSync(join(pkgDir, "bin"), { recursive: true });
  cpSync(src, join(pkgDir, "bin", binName));
  if (t.os !== "win32") chmodSync(join(pkgDir, "bin", binName), 0o755);
  writeFileSync(
    join(pkgDir, "package.json"),
    JSON.stringify(
      {
        name: `@claudinio/${pkgName}`,
        version,
        description: `Claudinio Code CLI — binário pré-compilado (${t.os}-${t.cpu})`,
        os: [t.os],
        cpu: [t.cpu],
        files: ["bin/"],
        license: "MIT",
      },
      null,
      2
    ) + "\n"
  );
  built.push(t);
}

// Launcher: copia o template e injeta versão + optionalDependencies só dos
// targets efetivamente buildados.
const launcherDir = join(distRoot, "claudinio");
cpSync(join(root, "npm/claudinio"), launcherDir, { recursive: true });
const launcher = JSON.parse(readFileSync(join(launcherDir, "package.json"), "utf8"));
launcher.version = version;
launcher.optionalDependencies = Object.fromEntries(
  built.map((t) => [`@claudinio/cli-${t.os}-${t.cpu}`, version])
);
writeFileSync(join(launcherDir, "package.json"), JSON.stringify(launcher, null, 2) + "\n");

console.log(
  `Montados ${built.length} pacote(s) de plataforma + launcher em npm/dist (v${version}).`
);
