# Exemplos de Diagramas Mermaid para Testar no Chat

Copie e cole um destes blocos no chat do Claudinio Code para testar a renderização:

---

## Exemplo 1: Fluxograma simples

```mermaid
graph TD
    A[Início] --> B{Decisão}
    B -->|Sim| C[Processar]
    B -->|Não| D[Sair]
    C --> E[Fim]
```

---

## Exemplo 2: Diagrama de sequência

```mermaid
sequenceDiagram
    participant U as Usuário
    participant C as Chat
    participant A as Agente
    U->>C: Digita mensagem
    C->>A: Envia prompt
    A->>C: Responde com código
    C->>U: Exibe resposta
```

---

## Exemplo 3: Diagrama de classes

```mermaid
classDiagram
    class ChatPanel {
        +messages()
        +sendMessage()
        -viewerFile()
    }
    class DiffViewer {
        +original: string
        +modified: string
    }
    class ContentViewerModal {
        +contentType
        +filePath
    }
    ChatPanel --> DiffViewer
    ChatPanel --> ContentViewerModal
```

---

## Exemplo 4: Diagrama de Gantt (cronograma)

```mermaid
gantt
    title Ciclo de Desenvolvimento
    dateFormat  YYYY-MM-DD
    section Planejamento
    Análise           :done, 2026-07-01, 7d
    Design            :active, 2026-07-08, 5d
    section Implementação
    Frontend          :2026-07-13, 10d
    Backend           :2026-07-15, 8d
    Testes            :2026-07-23, 5d
```

---

## Instruções

1. Copie qualquer bloco ```mermaid ...``` acima
2. Cole no campo de texto do chat
3. Envie a mensagem
4. O diagrama deve aparecer renderizado inline
5. Clique no diagrama para abrir o visualizador em tela cheia (zoom/pan)
