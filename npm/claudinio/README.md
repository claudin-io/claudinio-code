# claudinio

CLI do **Claudinio Code** — agente de código no terminal (brain/builder, busca
semântica híbrida), reaproveitando o mesmo backend Rust do app desktop. Sem
JS/webview.

## Uso

```bash
npx claudinio auth login
npx claudinio index .
npx claudinio search "hybrid retrieval"
npx claudinio run -m brain "explique o módulo X"
npx claudinio chat        # TUI interativa
```

Ou instale globalmente:

```bash
npm i -g claudinio
claudinio --help
```

## Como funciona

Este pacote é um launcher fino: ele resolve, em tempo de execução, o binário
nativo pré-compilado do pacote de plataforma correspondente
(`@claudinio/cli-<os>-<cpu>`), instalado automaticamente pelo npm via
`optionalDependencies` (filtrado por `os`/`cpu`). Nenhuma compilação ocorre no
`install`.

O modelo de embeddings (~23 MB) é baixado no primeiro uso que precisar de busca
vetorial e fica em cache no diretório de dados do app.

Plataformas suportadas: `darwin-arm64`, `linux-x64`, `linux-arm64`,
`win32-x64`, `win32-arm64`.
