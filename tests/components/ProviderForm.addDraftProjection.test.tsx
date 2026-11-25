/**
 * 新增对话框：Codex、Gemini CLI、Grok Build 的表单把预设或模板投影到当前配置文件上显示，
 * 投影结果就是保存时三方比较的底。锁定一条不变式：每次重置显示内容之后，编辑框里的内容
 * 和最后上报的底一致。对不上时，底里有、显示里没有的全局设置会在保存时被当成删除写进
 * 配置文件。
 *
 * MSW 替身：投影在原内容上加一份「配置文件里已有的全局设置」，模拟真实的 live。
 */
import {
  act,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/react";
import { QueryClientProvider } from "@tanstack/react-query";
import { http, HttpResponse } from "msw";
import { beforeEach, describe, expect, it, vi } from "vitest";
import {
  ProviderForm,
  type ProviderFormProps,
} from "@/components/providers/forms/ProviderForm";
import { codexProviderPresets } from "@/config/codexProviderPresets";
import { geminiProviderPresets } from "@/config/geminiProviderPresets";
import { grokBuildProviderPresets } from "@/config/grokBuildProviderPresets";
import { providersApi, type AppId } from "@/lib/api";
import type { ProviderEditorView } from "@/lib/api/providers";
import { server } from "../msw/server";
import { createTestQueryClient } from "../utils/testQueryClient";

vi.mock("@/components/JsonEditor", () => ({
  default: ({
    value,
    onChange,
    ariaLabel,
  }: {
    ariaLabel?: string;
    value: string;
    onChange: (value: string) => void;
  }) => (
    <textarea
      aria-label={ariaLabel}
      data-testid="json-editor"
      value={value}
      onChange={(event) => onChange(event.target.value)}
    />
  ),
}));

vi.mock("@/components/providers/forms/CodexOAuthSection", () => ({
  CodexOAuthSection: () => <div data-testid="codex-oauth-section" />,
}));

vi.mock("@/components/providers/forms/hooks", async (importOriginal) => {
  const actual =
    await importOriginal<typeof import("@/components/providers/forms/hooks")>();
  return {
    ...actual,
    useCopilotAuth: () => ({
      isAuthenticated: false,
      isStatusSuccess: true,
      isStatusError: false,
      accounts: [],
    }),
    useCodexOauth: () => ({
      isAuthenticated: false,
      isStatusSuccess: true,
      isStatusError: false,
      defaultAccountId: null,
      accounts: [],
    }),
    useXaiOauth: () => ({ isAuthenticated: false, accounts: [] }),
  };
});

const LIVE_TOML = '\n[ui]\ntheme = "dark"\n';

type Base = Record<string, unknown> | null;

function projectAsLive() {
  server.use(
    http.post(
      "http://tauri.local/get_provider_editor_view",
      async ({ request }) => {
        const { app, settingsConfig = {} } = (await request.json()) as {
          app: AppId;
          settingsConfig?: Record<string, unknown>;
        };
        const settings = { ...settingsConfig };
        if (app === "gemini") {
          settings.env = {
            ...(settingsConfig.env as Record<string, unknown>),
            GEMINI_SANDBOX: "docker",
          };
          settings.config = {
            ...(settingsConfig.config as Record<string, unknown>),
            ui: { theme: "dark" },
          };
        } else {
          settings.config = `${String(settingsConfig.config ?? "")}${LIVE_TOML}`;
        }
        return HttpResponse.json({ settings, inactive: [] });
      },
    ),
  );
}

function renderForm(appId: AppId, bases: Base[]) {
  return render(
    <QueryClientProvider client={createTestQueryClient()}>
      <ProviderForm
        appId={appId}
        submitLabel="save"
        onSubmit={vi.fn()}
        onCancel={vi.fn()}
        onEditorBaseChange={(base) => bases.push(base)}
      />
    </QueryClientProvider>,
  );
}

function clickPreset(name: string) {
  const matches = screen
    .getAllByRole("button")
    .filter(
      (button) => button.querySelector("span.truncate")?.textContent === name,
    );
  expect(matches, `预设按钮「${name}」应唯一`).toHaveLength(1);
  fireEvent.click(matches[0]);
}

function editorTexts(): string[] {
  return screen
    .getAllByTestId("json-editor")
    .map((node) => (node as HTMLTextAreaElement).value);
}

/** 等到最后一次上报的底不为空，并且和上一次检查时不同。 */
async function nextBase(bases: Base[], seen: number): Promise<Base> {
  await waitFor(() => {
    expect(bases.length).toBeGreaterThan(seen);
    expect(bases.at(-1)).not.toBeNull();
  });
  return bases.at(-1)!;
}

describe("新增对话框的草稿投影", () => {
  beforeEach(() => {
    projectAsLive();
  });

  it("Codex：打开、选预设、切回自定义，显示内容始终等于底", async () => {
    const bases: Base[] = [];
    renderForm("codex", bases);

    let base = await nextBase(bases, 0);
    await waitFor(() => expect(editorTexts()).toContain(String(base!.config)));

    const preset = codexProviderPresets.find(
      (item) => item.category !== "official" && item.name === "Nvidia",
    )!;
    let seen = bases.length;
    clickPreset(preset.name);
    base = await nextBase(bases, seen);
    expect(String(base!.config)).toContain(LIVE_TOML.trim());
    await waitFor(() => expect(editorTexts()).toContain(String(base!.config)));

    seen = bases.length;
    clickPreset("providerPreset.custom");
    base = await nextBase(bases, seen);
    expect(String(base!.config)).not.toContain("nvidia");
    await waitFor(() => expect(editorTexts()).toContain(String(base!.config)));
  });

  it("Gemini：选预设、切回自定义后，全局设置仍在显示内容和底里", async () => {
    const bases: Base[] = [];
    renderForm("gemini", bases);
    await nextBase(bases, 0);

    const preset = geminiProviderPresets.find(
      (item) => item.category !== "official",
    )!;
    let seen = bases.length;
    clickPreset(preset.name);
    let base = await nextBase(bases, seen);
    expect((base!.env as Record<string, unknown>).GEMINI_SANDBOX).toBe(
      "docker",
    );

    seen = bases.length;
    clickPreset("providerPreset.custom");
    base = await nextBase(bases, seen);
    expect((base!.env as Record<string, unknown>).GEMINI_SANDBOX).toBe(
      "docker",
    );
    await waitFor(() =>
      expect(
        editorTexts().some((text) => text.includes("GEMINI_SANDBOX=docker")),
      ).toBe(true),
    );
  });

  it("Grok Build：选预设后显示投影内容，切到官方卡作废底", async () => {
    const bases: Base[] = [];
    renderForm("grokbuild", bases);
    await nextBase(bases, 0);

    const preset = grokBuildProviderPresets[0];
    const seen = bases.length;
    clickPreset(preset.nameKey ?? preset.name);
    const base = await nextBase(bases, seen);
    await waitFor(() => expect(editorTexts()).toContain(String(base!.config)));

    fireEvent.click(screen.getByRole("button", { name: /Grok Official/ }));
    await waitFor(() => expect(bases.at(-1)).toBeNull());
  });
});

describe("完整 Codex TOML", () => {
  const name = "custom full config";
  const raw =
    '# relay profile\nmodel_provider = "team"\n[model_providers.team]\nname = "Team “One”"\nenv_key = "TEAM_KEY"\n';
  const settings = {
    auth: { OPENAI_API_KEY: "saved" },
    config: raw,
    modelCatalog: { models: [{ model: "catalog-only" }] },
  };
  const fullToggle = () =>
    screen.getByRole("checkbox", { name: "codexConfig.fullConfig" });
  const fullEditor = () => screen.getByRole("textbox", { name: "config.toml" });

  function renderCodex(props: Partial<ProviderFormProps> = {}) {
    const onSubmit = vi.fn();
    const client = createTestQueryClient();
    render(
      <QueryClientProvider client={client}>
        <ProviderForm
          appId="codex"
          submitLabel="save"
          onCancel={vi.fn()}
          onSubmit={onSubmit}
          {...props}
        />
      </QueryClientProvider>,
    );
    return { onSubmit, client };
  }

  it("loads current disk text when converting an active guided card and retains that snapshot as its edit base", async () => {
    const projected = raw + LIVE_TOML;
    const disk = raw + "\n# changed externally since the card was opened\n";
    const base = { ...settings, config: disk };
    const read = vi
      .spyOn(providersApi, "getEditorView")
      .mockRejectedValueOnce(new Error("read denied"))
      .mockResolvedValueOnce({ settings: base, inactive: [] });
    try {
      const { onSubmit } = renderCodex({
        providerId: "active-team",
        storedCodexConfig: raw,
        initialData: {
          name,
          category: "third_party",
          settingsConfig: { ...settings, config: projected },
        },
      });
      fireEvent.click(fullToggle());
      expect(await screen.findByRole("alert")).toHaveTextContent("read denied");
      expect(fullToggle()).not.toBeChecked();
      expect(editorTexts()).toContain(projected);
      fireEvent.click(screen.getByRole("button", { name: "common.retry" }));
      expect(
        await screen.findByRole("textbox", { name: "config.toml" }),
      ).toHaveValue(disk);
      expect(read).toHaveBeenLastCalledWith(
        "codex",
        settings,
        "third_party",
        "active-team",
        { codexFullConfig: true },
      );
      const edited = disk + "\n[features]\nplugins = false\n";
      fireEvent.change(fullEditor(), { target: { value: edited } });
      fireEvent.click(fullToggle());
      expect(editorTexts()).toContain(projected);
      fireEvent.click(fullToggle());
      expect(fullEditor()).toHaveValue(edited);
      fireEvent.click(screen.getByRole("button", { name: "save" }));
      await waitFor(() => expect(onSubmit).toHaveBeenCalledTimes(1));
      const result = onSubmit.mock.calls[0][0];
      expect(JSON.parse(result.settingsConfig)).toEqual({
        ...settings,
        config: edited,
      });
      expect(result.meta.codexFullConfig).toBe(true);
      expect(result.codexEditorBase).toEqual(base);
      expect(read).toHaveBeenCalledTimes(2);
    } finally {
      read.mockRestore();
    }
  });

  it("starts blank without a template, validates TOML on submit and keeps explicit empty files", async () => {
    const { onSubmit } = renderCodex({ modeView: "stack" });
    await screen.findByRole("checkbox", { name: "codexConfig.fullConfig" });
    fireEvent.click(fullToggle());
    expect(fullEditor()).toHaveValue("");
    fireEvent.change(screen.getByPlaceholderText("provider.namePlaceholder"), {
      target: { value: name },
    });
    fireEvent.change(fullEditor(), { target: { value: "[broken" } });
    fireEvent.click(screen.getByRole("button", { name: "save" }));
    await screen.findByRole("alert");
    expect(onSubmit).not.toHaveBeenCalled();
    fireEvent.change(fullEditor(), { target: { value: "" } });
    fireEvent.click(screen.getByRole("button", { name: "save" }));
    await waitFor(() => expect(onSubmit).toHaveBeenCalledTimes(1));
    expect(JSON.parse(onSubmit.mock.calls[0][0].settingsConfig).config).toBe(
      "",
    );
    expect(onSubmit.mock.calls[0][0].meta.codexFullConfig).toBe(true);
  });

  it("keeps the raw draft when an earlier guided projection arrives", async () => {
    let resolveView!: (view: ProviderEditorView) => void;
    const loaded = new Promise<ProviderEditorView>((resolve) => {
      resolveView = resolve;
    });
    const getView = vi
      .spyOn(providersApi, "getEditorView")
      .mockReturnValueOnce(loaded);
    const reportBase = vi.fn();
    renderCodex({ onEditorBaseChange: reportBase });
    await waitFor(() => expect(getView).toHaveBeenCalled());
    fireEvent.click(fullToggle());
    fireEvent.change(fullEditor(), { target: { value: raw } });
    const shown = { ...settings, config: raw + LIVE_TOML };
    await act(async () => {
      resolveView({ settings: shown, inactive: [] });
    });
    expect(fullEditor()).toHaveValue(raw);
    fireEvent.click(fullToggle());
    expect(editorTexts()).toContain(shown.config);
    expect(reportBase).toHaveBeenLastCalledWith(shown, expect.anything());
    getView.mockRestore();
  });

  it.each(["", raw])(
    "seeds a new full file from disk before the personal template",
    async (disk) => {
      server.use(
        http.post("http://tauri.local/get_settings", () =>
          HttpResponse.json({ codexCustomTemplate: "# personal" }),
        ),
      );
      const read = vi
        .spyOn(providersApi, "getCodexLocalConfig")
        .mockResolvedValue(disk);
      try {
        renderCodex();
        expect(
          await screen.findByRole("textbox", { name: "config.toml" }),
        ).toHaveValue(disk);
        expect(fullToggle()).toBeChecked();
      } finally {
        read.mockRestore();
      }
    },
  );

  it("takes a fresh disk snapshot on explicitly reselecting custom", async () => {
    const changed = "# edited outside CC Switch\n";
    const read = vi
      .spyOn(providersApi, "getCodexLocalConfig")
      .mockResolvedValueOnce(raw)
      .mockResolvedValueOnce(changed);
    try {
      renderCodex();
      await screen.findByRole("textbox", { name: "config.toml" });
      fireEvent.change(fullEditor(), { target: { value: "# unsaved draft" } });
      clickPreset("Nvidia");
      clickPreset("providerPreset.custom");
      await waitFor(() => expect(fullEditor()).toHaveValue(changed));
    } finally {
      read.mockRestore();
    }
  });

  it("reports a disk read failure and retries without saving the template", async () => {
    server.use(
      http.post("http://tauri.local/get_settings", () =>
        HttpResponse.json({ codexCustomTemplate: "# personal" }),
      ),
    );
    const read = vi
      .spyOn(providersApi, "getCodexLocalConfig")
      .mockRejectedValueOnce(new Error("permission denied"))
      .mockResolvedValueOnce(raw);
    try {
      const { onSubmit } = renderCodex();
      expect(await screen.findByRole("alert")).toHaveTextContent(
        "permission denied",
      );
      fireEvent.click(screen.getByRole("button", { name: "save" }));
      expect(onSubmit).not.toHaveBeenCalled();
      fireEvent.click(screen.getByRole("button", { name: "common.retry" }));
      expect(
        await screen.findByRole("textbox", { name: "config.toml" }),
      ).toHaveValue(raw);
    } finally {
      read.mockRestore();
    }
  });

  it("ignores a delayed disk read after the user picks a different preset", async () => {
    let resolveRead!: (text: string) => void;
    const loaded = new Promise<string>((resolve) => {
      resolveRead = resolve;
    });
    const read = vi
      .spyOn(providersApi, "getCodexLocalConfig")
      .mockReturnValueOnce(loaded);
    try {
      renderCodex();
      await waitFor(() => expect(read).toHaveBeenCalled());
      clickPreset("Nvidia");
      await act(async () => {
        resolveRead(raw);
      });
      expect(
        screen.queryByRole("textbox", { name: "config.toml" }),
      ).not.toBeInTheDocument();
      expect(editorTexts().some((text) => text.includes("nvidia"))).toBe(true);
    } finally {
      read.mockRestore();
    }
  });

  it("opens an existing full card with the editor snapshot even when its stored text is older", () => {
    renderCodex({
      providerId: "team",
      storedCodexConfig: "# stale",
      initialData: {
        name,
        category: "third_party",
        meta: { codexFullConfig: true },
        settingsConfig: settings,
      },
    });
    expect(fullEditor()).toHaveValue(raw);
  });

  it("loads the personal template once and reapplies it only when custom is reselected", async () => {
    server.use(
      http.post("http://tauri.local/get_settings", () =>
        HttpResponse.json({ codexCustomTemplate: raw }),
      ),
    );
    const { client } = renderCodex();
    await screen.findByRole("textbox", { name: "config.toml" });
    expect(fullToggle()).toBeChecked();
    expect(fullEditor()).toHaveValue(raw);
    fireEvent.change(fullEditor(), {
      target: { value: "# keeping my draft\n" },
    });
    server.use(
      http.post("http://tauri.local/get_settings", () =>
        HttpResponse.json({ codexCustomTemplate: "" }),
      ),
    );
    await act(async () => {
      await client.invalidateQueries({ queryKey: ["settings"] });
    });
    expect(client.getQueryData(["settings"])).toMatchObject({
      codexCustomTemplate: "",
    });
    expect(fullEditor()).toHaveValue("# keeping my draft\n");
    clickPreset("Nvidia");
    clickPreset("providerPreset.custom");
    await waitFor(() => expect(fullEditor()).toHaveValue(""));
    expect(fullToggle()).toBeChecked();
  });

  it("acquires a guided base when leaving a previously saved complete file", async () => {
    projectAsLive();
    const { onSubmit } = renderCodex({
      providerId: "team",
      initialData: {
        name,
        category: "third_party",
        meta: { codexFullConfig: true },
        settingsConfig: {
          ...settings,
          config: raw + 'base_url = "https://team.invalid/v1"\n',
        },
      },
    });
    expect(fullToggle()).toBeChecked();
    fireEvent.click(fullToggle());
    await waitFor(() => expect(fullToggle()).not.toBeChecked());
    fireEvent.click(screen.getByRole("button", { name: "save" }));
    await waitFor(() => expect(onSubmit).toHaveBeenCalledTimes(1));
    const payload = onSubmit.mock.calls[0][0];
    expect(payload.meta.codexFullConfig).toBeUndefined();
    expect(payload.codexEditorBase.config).toContain(LIVE_TOML);
    expect(JSON.parse(payload.settingsConfig).config).toContain(LIVE_TOML);
  });
});
