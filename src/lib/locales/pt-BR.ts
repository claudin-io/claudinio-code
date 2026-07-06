import type { LocaleDict } from "../grill-me";

const dict: LocaleDict = {
  // ── App ───────────────────────────────────────────────────────────
  "app.title": "Claudinio Code",
  "app.config.title": "Configuração da API",
  "app.config.apiKey": "API Key",
  "app.config.baseUrl": "Base URL",
  "app.config.model": "Modelo",
  "app.config.cancel": "Cancelar",
  "app.config.save": "Salvar",
  "app.sidebar.projects": "Projetos",
  "app.sidebar.noRecent": "Nenhum projeto recente",
  "app.sidebar.openFolder": "Abrir pasta",
  "app.sidebar.browseFiles": "Explorar arquivos",
  "app.sidebar.back": "Voltar",
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

  // ── ChatPanel - Archived ──────────────────────────────────────────
  "chat.archived.title": "Histórico compactado",
  "chat.archived.messages": "{0} mensagens",
  "chat.archived.you": "Você",
  "chat.archived.agent": "Agente",

  // ── ChatPanel - Drop overlay ──────────────────────────────────────
  "chat.drop.title": "Solte o arquivo para anexar",
  "chat.drop.hint": "Imagens, PDFs, documentos, código e mais",
};

export default dict;
