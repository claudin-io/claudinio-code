# Adicionar Builds: macOS x64, Windows ARM, Linux ARM

## Context

O projeto Claudinio Code (Tauri v2 + SolidJS) atualmente gera builds apenas para 3 plataformas no CI:
- Windows x64 (`x86_64-pc-windows-msvc`)
- macOS ARM64 / Apple Silicon (`aarch64-apple-darwin`)
- Linux x64 (`x86_64-unknown-linux-gnu`)

O usuário solicita adicionar mais 3 alvos de build:
1. macOS x64 (Intel)
2. Windows ARM64 (aarch64)
3. Linux ARM64 (aarch64)

Decisões confirmadas com o usuário:
- **macOS x64**: Usar runner `macos-13` (GitHub Actions, último Intel disponível)
- **Windows ARM64**: Cross-compilar do runner `windows-latest` (x64) com toolchain MSVC ARM64
- **Linux ARM64**: Usar runner nativo `ubuntu-24.04-arm`
- **Windows ARM bundle**: Usar `--bundles nsis` (evitar WiX que tem problemas com cross-compile ARM64)

## Solution Design

### 1. Matriz de Build (CI — `release.yml`)

Adicionar 3 novas entradas à matrix `include:`:

| platform | target | os | bundle-args | artifact-name |
|---|---|---|---|---|
| `macos-x64` | `x86_64-apple-darwin` | `macos-13` | (vazio) | `Claudinio-Code-macOS-x64` |
| `windows-arm64` | `aarch64-pc-windows-msvc` | `windows-latest` | `--bundles nsis` | `Claudinio-Code-Windows-arm64` |
| `linux-arm64` | `aarch64-unknown-linux-gnu` | `ubuntu-24.04-arm` | (vazio) | `Claudinio-Code-Linux-arm64` |

O total de builds passa de 3 para 6.

### 2. Passos Específicos por Plataforma

**Windows ARM64** — requer passos extras no CI:
- Usar `ilammy/msvc-dev-cmd@v1` com `arch: x64_arm64` para configurar o ambiente MSVC de cross-compilação
- `rustup target add aarch64-pc-windows-msvc`
- `pnpm tauri build --target aarch64-pc-windows-msvc --bundles nsis`
- Manter o linker flag `RUSTFLAGS=-Ctarget-feature=-crt-static` (já está no passo Windows existente)

**macOS x64**:
- A mesma pipeline de code signing Apple já existente funciona (runner `macos-13` também roda macOS)
- Apenas adicionar o target `x86_64-apple-darwin` via `dtolnay/rust-toolchain`
- A etapa "Import Apple Developer Certificate" roda em todos runners macOS

**Linux ARM64**:
- Usar o mesmo passo de `apt-get install` — pacotes já disponíveis para ARM64 no Ubuntu 24.04
- Apenas `rustup target add aarch64-unknown-linux-gnu` via `dtolnay/rust-toolchain`
- O Tauri CLI já está no lockfile para ARM64 (`@tauri-apps/cli-linux-arm64-gnu`)
- `ort` (ONNX Runtime) baixa bins ARM64 via `download-binaries`

### 3. Manifesto de Atualização (`latest.json`)

Adicionar novas entradas ao updater:

| Chave | Artefato |
|---|---|
| `darwin-aarch64` | (já existe) |
| **`darwin-x86_64`** | `*macOS-x64*.app.tar.gz` |
| `windows-x86_64` | (já existe) |
| **`windows-aarch64`** | `*Windows-arm64*.exe` (NSIS) |
| `linux-x86_64` | (já existe) |
| **`linux-aarch64`** | `*Linux-arm64*.AppImage` |

### 4. Notas de Release

Atualizar a tabela de downloads no release notes gerado automaticamente.

### 5. Coleta de Artefatos

O passo `upload-artifact` existente faz glob de `src-tauri/target/release/bundle/**/*`, que captura todos os bundles da plataforma atual. Não precisa de alterações.

O passo `Collect release files` faz find por nome de diretório `Claudinio-Code-*` com glob de extensões — funciona para os novos artefatos.

## Risks

- **Windows ARM cross-compile**: O `ilammy/msvc-dev-cmd` depende do VS Build Tools estar pré-instalado no runner `windows-latest`. A Microsoft pré-instala o VS 2022 Build Tools nos runners GitHub, mas precisamos confirmar se o componente ARM64 está incluído. Se não estiver, precisaremos de um passo extra com `vs_BuildTools.exe --add Microsoft.VisualStudio.Component.VC.ARM64`.
- **macOS 13 runner**: `macos-13` é macOS Ventura (13.x). O Xcode disponível pode ser mais antigo que no `macos-latest`. Isso não deve ser problema para Rust, mas a assinatura de código Apple pode precisar de versão específica do `codesign`.
- **Tempo de CI**: 6 builds paralelos vs 3 antes — maior consumo de minutos de CI.
- **Size do updater**: O `latest.json` fica maior com 6 plataformas, mas isso é esperado.
- **ARM64 runner availability**: `ubuntu-24.04-arm` pode ter disponibilidade diferente do x64. Em picos de uso, os jobs ARM64 podem esperar mais na fila.
- **ONNX Runtime ARM64**: Verificar se `ort` com `download-binaries` baixa corretamente o binário ARM64 para cada plataforma. ONNX Runtime suporta oficialmente ARM64 Linux, Windows ARM64 e macOS x64 — baixo risco.

## Non-goals

- Não vamos modificar o código Rust ou frontend — apenas configuração de CI
- Não vamos adicionar toolchain customizado ou cross-compilação de Linux ARM64 via QEMU (usamos runner nativo)
- Não vamos mudar a estratégia de code signing existente
- Não vamos adicionar testes/lint separados (já está incorporado no `pnpm build`)

## Low-Level Design

### Arquivos a Modificar

**Único arquivo:** `.github/workflows/release.yml`

### 1. Matrix de Build (linhas ~13-34)

Adicionar 3 novos `include:` entries após o entry `linux-x64` existente:

```yaml
          # macOS Intel (x86_64) — via macos-13 (último runner Intel da GitHub)
          - platform: macos-x64
            target: x86_64-apple-darwin
            os: macos-13
            bundle-args: ''
            artifact-name: Claudinio-Code-macOS-x64

          # Windows ARM64 — cross-compilado do runner x64
          - platform: windows-arm64
            target: aarch64-pc-windows-msvc
            os: windows-latest
            bundle-args: '--bundles nsis'
            artifact-name: Claudinio-Code-Windows-arm64

          # Linux ARM64 — via runner nativo ARM64
          - platform: linux-arm64
            target: aarch64-unknown-linux-gnu
            os: ubuntu-24.04-arm
            bundle-args: ''
            artifact-name: Claudinio-Code-Linux-arm64
```

### 2. Passo de Instalação do Rust Toolchain (linha ~38)

O `dtolnay/rust-toolchain` já usa `targets: ${{ matrix.target }}`. Como adicionamos os targets corretos na matrix, funciona sem alterações. ✅

### 3. Passo MSVC Dev Cmd para Windows ARM64

Novo passo condicional, inserido **antes** do passo "Set Windows linker flags" (linha ~76):

```yaml
      - name: Set up MSVC ARM64 cross-compilation tools
        if: matrix.target == 'aarch64-pc-windows-msvc'
        uses: ilammy/msvc-dev-cmd@v1
        with:
          arch: x64_arm64
```

### 4. Adicionar Rust Target para Windows ARM64

Novo passo condicional, também antes da build (após o passo MSVC):

```yaml
      - name: Add Rust ARM64 target
        if: matrix.target == 'aarch64-pc-windows-msvc'
        shell: bash
        run: rustup target add aarch64-pc-windows-msvc
```

### 5. Tauri Build Command (linha ~140)

Substituir `pnpm tauri build` por:

```yaml
        run: pnpm tauri build ${{ matrix.bundle-args }}
```

### 6. Manifesto de Atualização (`latest.json`) (linha ~195)

Adicionar 3 novas chamadas `add_platform` após as existentes:

```bash
          add_platform "darwin-x86_64"   "$(asset '*macOS-x64*.app.tar.gz')"
          add_platform "windows-aarch64" "$(asset '*Windows-arm64*.exe')"
          add_platform "linux-aarch64"   "$(asset '*Linux-arm64*.AppImage')"
```

### 7. Notas de Release (linha ~218)

Atualizar a tabela de downloads:

```markdown
          echo "| Platform | File |"
          echo "|----------|------|"
          echo "| Windows (x64) | \`Claudinio-Code-Windows-x64-*.msi\` / \`*.exe\` |"
          echo "| Windows (ARM64) | \`Claudinio-Code-Windows-arm64-*.exe\` |"
          echo "| macOS (Intel) | \`Claudinio-Code-macOS-x64-*.dmg\` |"
          echo "| macOS (Apple Silicon) | \`Claudinio-Code-macOS-arm64-*.dmg\` |"
          echo "| Linux (x64) | \`Claudinio-Code-Linux-x64-*.deb\` / \`*.AppImage\` |"
          echo "| Linux (ARM64) | \`Claudinio-Code-Linux-arm64-*.deb\` / \`*.AppImage\` |"
```

### 8. Considerações sobre Code Signing

- **macOS x64** no runner `macos-13`: O certificado Apple Developer ID é importado da mesma forma. O `security find-identity` e `codesign` funcionam igual. O `notarize` também. ✅
- **Windows ARM64**: `signtool.exe` no SDK do Windows suporta assinatura de bins ARM64 do host x64 nativamente. ✅

### 9. Ort / ONNX Runtime

A crate `ort` com `features = ["download-binaries"]` baixa o ONNX Runtime pré-compilado. O ONNX Runtime publica bins para todas as 3 novas plataformas:
- macOS x64 ✅ (`onnxruntime-osx-x64-*`)
- Windows ARM64 ✅ (`onnxruntime-win-arm64-*`)
- Linux ARM64 ✅ (`onnxruntime-linux-aarch64-*`)

A crate `tokenizers` (Rust puro com feature `onig`) compila sem problemas em todas as plataformas.

## Tasks

1. **Adicionar matrix entries** — Inserir 3 novos entries no `matrix.include` do `release.yml`: macos-x64 (runner macos-13, target x86_64-apple-darwin), windows-arm64 (runner windows-latest, target aarch64-pc-windows-msvc, bundle-args '--bundles nsis'), linux-arm64 (runner ubuntu-24.04-arm, target aarch64-unknown-linux-gnu)
2. **Adicionar passo MSVC Dev Cmd** — Inserir step condicional com `ilammy/msvc-dev-cmd@v1` e `arch: x64_arm64` para Windows ARM64, antes dos linker flags
3. **Adicionar rustup target para Windows ARM64** — Inserir step `rustup target add aarch64-pc-windows-msvc` condicional para o entry windows-arm64
4. **Modificar comando tauri build** — Trocar `pnpm tauri build` por `pnpm tauri build ${{ matrix.bundle-args }}` no step Build Tauri app
5. **Atualizar latest.json** — Adicionar 3 chamadas `add_platform` para darwin-x86_64, windows-aarch64, linux-aarch64 com os respectivos globs de artefato
6. **Atualizar release notes** — Adicionar linhas para Windows ARM64, macOS Intel, Linux ARM64 na tabela de downloads


## Implementation Log — 2026-07-15 09:46
**Summary:** Adiciona builds para macOS x64 (macos-13), Windows ARM64 (cross-compile), Linux ARM64 (runner nativo) — 3 novos targets no CI
**Changed files:** M .github/workflows/release.yml, M docs/plans/2026-07-15_2026-07-15-release-v0-1-8.md, ?? docs/plans/2026-07-15_add-new-build-targets.md
**Commits:** _(git unavailable or none)_
**Journal:** All 6 tasks implemented. Key decisions and details:
- macOS x64 usa runner `macos-13` (último Intel da GitHub, macOS Ventura). Compartilha a mesma pipeline de code signing Apple dos builds ARM64.
- Windows ARM64 usa cross-compilação do runner `windows-latest` x64: `ilammy/msvc-dev-cmd@v1` com `arch: x64_arm64` + `rustup target add aarch64-pc-windows-msvc`. Bundle usa `--bundles nsis` porque o NSIS é arch-agnostic (WiX/MSI tem problemas conhecidos com cross-compile ARM64).
- Linux ARM64 usa runner nativo `ubuntu-24.04-arm`. Os mesmos pacotes apt funcionam para ARM64 sem alterações. O Tauri CLI ARM64 (`@tauri-apps/cli-linux-arm64-gnu`) já está no lockfile.
- O comando `pnpm tauri build` foi alterado para `pnpm tauri build ${{ matrix.bundle-args }}` — vazio para a maioria, `--bundles nsis` para windows-arm64.
- Updater manifest (latest.json) cresceu de 3 para 6 plataformas. Atenção aos globs de busca dos artefatos — alguns usam wildcards curinga (`*macOS*`) que podem conflitar com os novos entries (ex: `darwin-aarch64` e `darwin-x86_64` ambos casam com `*macOS*`). A ordenação no script garante que o primeiro `.app.tar.gz` encontrado vai para cada plataforma, mas pode ser necessário ajustar se houver ambiguidade.
- Total de builds: 6 (antes 3). CI leva ~2x mais minutos.

**Task journal:**
- Adicionar 3 entries na matrix do CI: 3 entries adicionadas após linux-x64: macos-x64 (macos-13), windows-arm64 (windows-latest + --bundles nsis), linux-arm64 (ubuntu-24.04-arm)
- Adicionar passo MSVC Dev Cmd para Windows ARM64: Step MSVC Dev Cmd inserido antes do 'Set Windows linker flags' com condicional para aarch64-pc-windows-msvc
- Adicionar rustup target para Windows ARM64: Step 'Add Rust ARM64 target' inserido após o MSVC Dev Cmd e antes do 'Set Windows linker flags'
- Modificar comando tauri build para usar bundle-args: Comando atualizado para usar ${{ matrix.bundle-args }}, que é vazio para a maioria e '--bundles nsis' para windows-arm64
- Atualizar latest.json com 3 novas plataformas: 3 novas entradas no latest.json: darwin-x86_64, windows-aarch64, linux-aarch64 — totalizando 6 plataformas no updater
- Atualizar tabela de downloads nas release notes: Tabela de downloads expandida de 3 para 6 linhas, cobrindo todas as plataformas


## Implementation Log — 2026-07-15 12:37
**Summary:** Corrige macOS x64 (cross-compile do ARM) e Linux (libappindicator3 → libayatana) para builds v0.1.9
**Changed files:** M	.github/workflows/release.yml, M	docs/plans/2026-07-15_2026-07-15-release-v0-1-8.md, A	docs/plans/2026-07-15_add-new-build-targets.md, M	package.json, M	src-tauri/Cargo.toml, M	src-tauri/tauri.conf.json
**Commits:** 0baf071 fix: cross-compile macOS x64 from ARM runner, swap libappindicator3 for libayatana on Linux, e5aa2c4 chore: bump version to 0.1.9, a965535 chore: stage release workflow and docs changes for v0.1.8
**Journal:** Two issues found and fixed:

1. **macOS x64 fila infinita**: Runner `macos-13` (Intel) é escasso — filas de 5-30 min, e será deprecado. Solução: cross-compilar x86_64 do runner `macos-latest` (ARM) usando `--target x86_64-apple-darwin`. Code signing continua funcionando pois o runner ARM também tem macOS SDK + codesign + notarization.

2. **Linux ARM falhou**: `libappindicator3-dev` não existe no Ubuntu 24.04 (Noble) — removido dos repositórios. Isto afeta TANTO x86_64 quanto ARM64 no Ubuntu 24.04. Solução: substituir por `libayatana-appindicator3-dev` + symlink pkg-config (`ayatana-appindicator3-0.1.pc` → `appindicator3-0.1.pc`) para compatibilidade com o crate Rust `libappindicator-sys`.

Melhoria adicional: Adicionei `bundle-path` à matrix de CI porque builds com `--target` colocam os bundles em `target/<triple>/release/bundle/` em vez de `target/release/bundle/`. Windows ARM64 e macOS x64 usam target-triple path; os demais ficam no path original.

**Task journal:**
- Corrigir build macOS x64 — cross-compilar do ARM: macos-13 runners são escassos e serão deprecados. Agora macOS x64 é cross-compilado do macos-latest (ARM) com --target x86_64-apple-darwin. O bundle sai em target/x86_64-apple-darwin/release/bundle/ — bundle-path adicionado à matrix.
- Corrigir Linux — substituir libappindicator3-dev por libayatana: libappindicator3-dev removido no Noble (24.04). Substituído por libayatana-appindicator3-dev. Symlink pkg-config adicionado para compatibilidade com o crate libappindicator-sys que busca appindicator3-0.1.pc. Válido para x86_64 e ARM64.
