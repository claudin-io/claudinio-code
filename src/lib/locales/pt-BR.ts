import type { LocaleDict } from "../grill-me";

const dict: LocaleDict = {
  // ── App ───────────────────────────────────────────────────────────
  "app.title": "Claudinio Code",
  "app.config.title": "Configuração da API",
  "app.config.apiKey": "API Key",
  "app.config.baseUrl": "Base URL",
  "app.config.model": "Modelo",
  "app.config.maxRounds": "Limite de rounds (agente principal)",
  "app.config.subMaxRounds": "Limite de rounds (subagentes)",
  "app.config.unlimited": "Ilimitado (padrão)",
  "app.config.maxRoundsHint": "Deixe vazio para ilimitado. Define o número máximo de ferramentas que o agente pode chamar por envio.",
  "app.config.subMaxRoundsHint": "Deixe vazio para ilimitado. Define o limite por subagente.",
  "app.config.cancel": "Cancelar",
  "app.config.save": "Salvar",
  "app.sidebar.projects": "Projetos",
  "app.sidebar.noRecent": "Nenhum projeto recente",
  "app.sidebar.openFolder": "Abrir pasta",
  "app.sidebar.browseFiles": "Explorar arquivos",
  "app.sidebar.back": "Voltar",
  "app.sidebar.closeWorkspace": "Fechar workspace",
  "app.index.loadingModel": "Carregando modelo…",
  "app.index.indexing": "Indexando",
  "app.index.generatingEmbeddings": "Gerando embeddings",
  "app.index.embeddingsReady": "Embeddings prontos",
  "app.index.embeddingFailed": "Falha nos embeddings — busca semântica indisponível",
  "app.index.symbols": "símbolos",
  "app.index.filesCount": "{0} arquivos, {1} símbolos",
  "app.index.indexingStatus": "indexando…",
  "app.index.embeddingStatus": "Gerando embeddings",
  "app.config.saveError": "Erro ao salvar config: {0}",
  "app.config.yoloMode": "⚡ Modo YOLO (auto-aprovar tudo)",
  "app.config.yoloModeHint": "Auto-aprova chamadas de ferramentas exceto as na blacklist abaixo.",
  "app.config.yoloBlacklist": "YOLO Blacklist (nomes separados por vírgula)",
  "app.config.yoloBlacklistHint": "Essas ferramentas ainda exigem aprovação manual mesmo com YOLO ativo. Ex: edit_file, bash",

  // ── EmptyState ────────────────────────────────────────────────────
  "empty.title": "Claudinio Code",
  "empty.subtitle": "Abra uma pasta de projeto para começar a usar o agente.",
  "empty.openFolder": "Abrir pasta",
  "empty.recent": "Recentes",

  // ── ChatPanel - Header ────────────────────────────────────────────
  "chat.header.agent": "Agente",
  "chat.header.newSession": "Nova sessão",
  "chat.header.new": "Nova",
  "chat.header.savedSessions": "Sessões salvas",
  "chat.header.history": "Histórico",
  "chat.header.noSessions": "Nenhuma sessão salva.",
  "chat.header.sessionTitle": "{0} · {1} turno{2}",
  "chat.header.turns": "turnos",
  "chat.header.turn": "turno",

  // ── ChatPanel - Status ────────────────────────────────────────────
  "chat.status.thinking": "Trabalhando",
  "chat.status.awaitingApproval": "Aguardando aprovação",
  "chat.status.awaitingInput": "Aguardando sua resposta",
  "chat.status.done": "Pronto",
  "chat.status.error": "Erro",
  "chat.status.idle": "Parado",

  // ── ChatPanel - Messages ──────────────────────────────────────────
  "chat.message.you": "Você",
  "chat.message.agent": "Agente",
  "chat.message.failedToSend": "Falha ao enviar: {0}",
  "chat.message.failedToReopen": "Falha ao reabrir: {0}",
  "chat.message.failedToCompact": "Compactação falhou: {0}",
  "chat.scrollToBottom": "Ir para o fim",

  // ── ChatPanel - Phases ────────────────────────────────────────────
  "chat.phase.plan": "Planejamento",
  "chat.phase.execute": "Execução",
  "chat.phase.summary": "Sumário",

  // ── ChatPanel - Timeline ──────────────────────────────────────────
  "chat.timeline.thought": "Pensou",
  "chat.timeline.steering": "orientação",
  "chat.timeline.waiting": "Aguardando resposta...",
  "chat.timeline.workedFor": "Trabalhou por {0}",
  "chat.timeline.steps": "{0} passo{1}",
  "chat.timeline.args": "Argumentos",
  "chat.timeline.result": "Resultado",

  // ── ChatPanel - Subagent ──────────────────────────────────────────
  "chat.subagent.running": "Trabalhando",
  "chat.subagent.completed": "{0} rounds",
  "chat.subagent.failed": "Falhou",
  "chat.subagent.interrupted": "Interrompido",
  "chat.subagent.maxRounds": "Limite de rounds",
  "chat.subagent.rounds": "rounds",
  "chat.subagent.title": "Subagent: {0}",

  // ── ChatPanel - Input ─────────────────────────────────────────────
  "chat.input.attachFile": "Anexar arquivo",
  "chat.input.compacting": "Compactando contexto…",
  "chat.input.approveFirst": "Aprove ou rejeite a edição primeiro…",
  "chat.input.answerFirst": "Responda as perguntas acima primeiro…",
  "chat.input.steerAgent": "Digite para orientar o agente… (Esc para pausar)",
  "chat.input.stop": "Parar",
  "chat.input.askCode": "Pergunte algo sobre o código…",
  "chat.input.dropToAttach": "Solte o arquivo para anexar",
  "chat.input.dropHint": "Imagens, PDFs, documentos, código e mais",

  // ── ChatPanel - Approval ──────────────────────────────────────────
  "chat.approval.proposedEdit": "Edição proposta",
  "chat.approval.bashCommand": "Comando bash",
  "chat.approval.approve": "Aprovar",
  "chat.approval.reject": "Rejeitar",
  "chat.approval.failed": "Aprovação falhou: {0}",
  "chat.approval.rejectFailed": "Rejeição falhou: {0}",

  // ── ChatPanel - Question ──────────────────────────────────────────
  "chat.question.needsAnswer": "O agente precisa da sua resposta",
  "chat.question.other": "Outra resposta…",
  "chat.question.typeAnswer": "Digite sua resposta…",
  "chat.question.submit": "Responder",
  "chat.question.answerFailed": "Envio de respostas falhou: {0}",

  // ── ChatPanel - Context Footer ────────────────────────────────────
  "chat.context.nextRequest": "Contexto da próxima requisição",
  "chat.context.sessionTokens": "Tokens acumulados da sessão",
  "chat.context.sessionCost": "Custo acumulado da sessão",
  "chat.context.total": "total: {0}",
  "chat.context.compact": "Compactar",
  "chat.context.compacting": "Compactando…",
  "chat.context.compactLabel": "{0} / {1}",

  // ── ChatPanel - Compaction ────────────────────────────────────────
  "chat.compact.start": "Contexto em ~{0}k/{1}k tokens — compactando…",
  "chat.compact.done": "Contexto compactado: ~{0}k → ~{1}k tokens.",
  "chat.compact.fail": "Falha na compactação: {0} — continuando com contexto cheio.",

  // ── ChatPanel - Archived ──────────────────────────────────────────
  "chat.archived.title": "Histórico compactado",
  "chat.archived.messages": "{0} mensagens",
  "chat.archived.you": "Você",
  "chat.archived.agent": "Agente",

  // ── ChatPanel - Drop overlay ──────────────────────────────────────
  "chat.drop.title": "Solte o arquivo para anexar",
  "chat.drop.hint": "Imagens, PDFs, documentos, código e mais",
  // ── Tasks Panel ──────────────────────────────────────────────
  "tasks.panel.title": "Tarefas",
  "tasks.panel.noTasks": "Nenhuma tarefa ainda — peça ao agente para criar",
  "tasks.panel.showDetails": "Mostrar detalhes",
  "tasks.panel.hideDetails": "Esconder detalhes",
  "tasks.panel.journal": "Diário",
  "tasks.panel.cycleStatus": "Alternar status",
  "tasks.panel.collapse": "Recolher",
  "tasks.panel.expand": "Mostrar tarefas",
  "tasks.panel.refresh": "Atualizar",
  "tasks.status.todo": "Pendente",
  "tasks.status.doing": "Fazendo",
  "tasks.status.done": "Concluído",

  // ── Context Warning ──────────────────────────────────────────
  "context.warning.title": "Orçamento de Contexto",
  "context.warning.noData": "Não foi possível carregar dados do contexto.",
  "context.warning.agentsFile": "Arquivo Injetado",
  "context.warning.size": "Tamanho",
  "context.warning.lines": "Linhas",
  "context.warning.estTokens": "Tokens estimados",
  "context.warning.issues": "Issues",
  "context.warning.issuesFound": "{0} issues encontradas — essas diretivas consomem tokens de contexto a cada turno.",
  "context.warning.skills": "Skills Instaladas",
  "context.warning.totalSkills": "Total de skills",
  "context.warning.skillTokens": "Custo combinado em tokens",
  "context.warning.hintAgents": "💡 O arquivo AGENTS.md/CLAUDE.md é injetado no início de cada novo chat. Arquivos grandes consomem uma parte significativa do orçamento de contexto. Considere cortar seções desnecessárias.",
  "context.warning.hintSkills": "💡 As skills são injetadas no system prompt como XML. Skills com SKILL.md grandes aumentam o custo base de contexto. Revise se todas as skills ainda são necessárias.",
};

export default dict;
