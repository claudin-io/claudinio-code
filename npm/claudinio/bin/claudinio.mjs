#!/usr/bin/env node
// Launcher do CLI `claudinio`: resolve o binário pré-compilado do pacote de
// plataforma correspondente (instalado via optionalDependencies pelo npm, que
// filtra por `os`/`cpu`) e o executa repassando argv/stdio. `stdio: "inherit"`
// dá ao binário o TTY real — essencial para a TUI (`claudinio chat`).
import { spawnSync } from "node:child_process";
import { createRequire } from "node:module";

const require = createRequire(import.meta.url);

const platform = process.platform; // "darwin" | "linux" | "win32" | ...
const arch = process.arch; // "arm64" | "x64" | ...
const pkg = `@claudinio/cli-${platform}-${arch}`;
const binName = platform === "win32" ? "claudinio.exe" : "claudinio";

let binPath;
try {
  binPath = require.resolve(`${pkg}/bin/${binName}`);
} catch {
  console.error(
    `claudinio: nenhum binário pré-compilado para ${platform}-${arch}.\n` +
      `Esperava o pacote ${pkg}. Plataformas suportadas: darwin-arm64, ` +
      `linux-x64, linux-arm64, win32-x64, win32-arm64.`
  );
  process.exit(1);
}

const result = spawnSync(binPath, process.argv.slice(2), { stdio: "inherit" });
if (result.error) {
  console.error(`claudinio: falha ao executar o binário: ${result.error.message}`);
  process.exit(1);
}
process.exit(result.status ?? 1);
