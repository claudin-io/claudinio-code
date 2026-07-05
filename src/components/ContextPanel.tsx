import { createResource, For, Show, type Component } from "solid-js";
import { fileOutline } from "../lib/ipc";

const KIND_ICONS: Record<string, string> = {
  function_declaration: "fn",
  function_item: "fn",
  function_definition: "fn",
  method_definition: "fn",
  class_declaration: "cls",
  class_definition: "cls",
  struct_item: "str",
  struct_declaration: "str",
  enum_item: "enm",
  enum_declaration: "enm",
  interface_declaration: "ifc",
  trait_item: "trt",
  protocol_declaration: "prt",
  type_alias_declaration: "typ",
  import: "imp",
  import_statement: "imp",
  use_declaration: "use",
};

const KIND_COLORS: Record<string, string> = {
  fn: "text-yellow-400",
  cls: "text-blue-400",
  str: "text-green-400",
  enm: "text-purple-400",
  ifc: "text-cyan-400",
  trt: "text-teal-400",
  prt: "text-indigo-400",
  typ: "text-orange-400",
  imp: "text-ink-muted",
  use: "text-ink-muted",
};

const kindIcon = (kind: string): string => KIND_ICONS[kind] ?? "sym";
const kindColor = (icon: string): string => KIND_COLORS[icon] ?? "text-ink-muted";

export const ContextPanel: Component<{ filePath: () => string | null }> = (props) => {
  const [symbols] = createResource(
    () => props.filePath(),
    (path) => fileOutline(path),
  );

  return (
    <div class="flex h-full flex-col bg-surface-0">
      <div class="border-b border-border-subtle px-3 py-1.5 text-xs font-semibold uppercase tracking-wide text-ink-muted">
        Símbolos
      </div>
      <Show
        when={props.filePath()}
        fallback={
          <div class="flex flex-1 items-center justify-center px-4 text-center text-sm text-ink-muted">
            Abra um arquivo para ver seus símbolos
          </div>
        }
      >
        <div class="flex-1 overflow-y-auto py-1">
          <Show
            when={symbols() && symbols()!.length > 0}
            fallback={
              <div class="px-3 py-2 text-xs text-ink-muted">
                {symbols.error ? "Erro ao carregar" : "Nenhum símbolo encontrado"}
              </div>
            }
          >
            <For each={symbols()}>
              {(sym) => {
                const icon = kindIcon(sym.kind);
                return (
                  <div class="flex items-center gap-2 px-3 py-0.5 text-xs hover:bg-surface-2">
                    <span class={`w-5 shrink-0 font-mono text-[10px] ${kindColor(icon)}`}>
                      {icon}
                    </span>
                    <span class="truncate text-ink">{sym.name}</span>
                    <span class="ml-auto shrink-0 text-ink-muted">{sym.startLine}</span>
                  </div>
                );
              }}
            </For>
          </Show>
        </div>
      </Show>
    </div>
  );
};
