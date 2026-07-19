import { Component, Show, type Accessor, type Setter } from "solid-js";
import { t } from "../../lib/grill-me";
import { Icon } from "../Icon";

interface SettingsAccountProps {
  accountLogin: Accessor<string | null>;
  hasApiKey: Accessor<boolean>;
  loggingIn: Accessor<boolean>;
  configApiKey: Accessor<string>;
  setConfigApiKey: Setter<string>;
  settingsApiKeyError: Accessor<string | null>;
  doLogin: () => void;
  doLogout: () => void;
  openSupportUrl: () => void;
}

export const SettingsAccount: Component<SettingsAccountProps> = (props) => {
  return (
    <>
      <label class="mb-1 block text-xs text-ink-muted">{t("app.config.account")}</label>
      <Show
        when={props.accountLogin() || props.hasApiKey()}
        fallback={
          <div class="mb-2 space-y-2">
            <button
              onClick={props.doLogin}
              disabled={props.loggingIn()}
              class="w-full rounded-md bg-accent p-2 text-sm font-semibold text-white hover:opacity-90 disabled:opacity-50"
            >
              {props.loggingIn() ? t("app.config.signingIn") : t("app.config.signIn")}
            </button>
            <label class="block text-xs text-ink-muted">{t("app.config.apiKey")}</label>
            <input
              type="password"
              value={props.configApiKey()}
              onInput={(e) => props.setConfigApiKey(e.currentTarget.value)}
              placeholder="sk-..."
              class="w-full rounded-md border border-border-subtle bg-surface-0 p-2 text-sm text-ink placeholder:text-ink-muted focus:border-accent focus:outline-none focus:ring-1 focus:ring-accent"
            />
            <Show when={props.settingsApiKeyError()}>
              <div class="text-sm text-red-400">{props.settingsApiKeyError()}</div>
            </Show>
          </div>
        }
      >
        <div class="mb-2 flex items-center justify-between rounded-md border border-border-subtle bg-surface-0 p-2 text-sm">
          <span class="truncate text-ink">{t("app.config.signedIn")}</span>
          <button
            onClick={props.doLogout}
            class="ml-2 shrink-0 text-xs text-ink-muted hover:text-ink hover:underline"
          >
            {t("app.config.signOut")}
          </button>
        </div>
      </Show>

      {/* Support */}
      <div class="mb-3">
        <button
          onClick={props.openSupportUrl}
          class="flex items-center gap-2 w-full rounded-md border border-border-subtle bg-surface-0 p-2 text-sm text-ink hover:bg-surface-2 hover:border-accent/40 transition-colors"
        >
          <Icon name="speech-balloon-alt" class="h-4 w-4 shrink-0" />
          <span>{t("app.config.support")}</span>
        </button>
      </div>
    </>
  );
};
