import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { CodexTemplateSettings } from "@/components/settings/CodexTemplateSettings";
import { getCodexCustomTemplate } from "@/config/codexTemplates";

vi.mock("react-i18next", () => ({
  useTranslation: () => ({ t: (key: string) => key }),
}));

describe("Codex default config.toml", () => {
  it("opens the built-in template, rejects invalid TOML and saves a replacement verbatim", async () => {
    const onSave = vi.fn().mockResolvedValue(true);
    render(<CodexTemplateSettings onSave={onSave} />);
    fireEvent.click(screen.getByRole("button", { name: "common.edit" }));
    const editor = screen.getByRole("textbox", {
      name: "settings.codexTemplate.title",
    });
    expect(editor).toHaveValue(getCodexCustomTemplate().config);

    fireEvent.change(editor, { target: { value: "[unclosed" } });
    fireEvent.click(screen.getByRole("button", { name: "common.save" }));
    expect(screen.getByRole("alert")).toBeInTheDocument();
    expect(onSave).not.toHaveBeenCalled();

    const custom = "# My profile\n[profiles.work]\nmodel = 'work-model'\n";
    fireEvent.change(editor, { target: { value: custom } });
    fireEvent.click(screen.getByRole("button", { name: "common.save" }));
    expect(onSave).toHaveBeenCalledWith(custom);
    await waitFor(() => expect(editor).not.toBeInTheDocument());
  });

  it("distinguishes a blank template from restoring the built-in default", async () => {
    const onSave = vi.fn().mockResolvedValue(false);
    render(<CodexTemplateSettings template="model = 'old'" onSave={onSave} />);
    fireEvent.click(screen.getByRole("button", { name: "common.edit" }));
    fireEvent.change(screen.getByRole("textbox"), { target: { value: "" } });
    fireEvent.click(screen.getByRole("button", { name: "common.save" }));
    expect(onSave).toHaveBeenCalledWith("");
    const restore = screen.getByRole("button", {
      name: "settings.codexTemplate.restore",
    });
    await waitFor(() => expect(restore).toBeEnabled());
    onSave.mockResolvedValue(true);
    fireEvent.click(restore);
    expect(onSave).toHaveBeenCalledWith(undefined);
    await waitFor(() => expect(screen.queryByRole("textbox")).toBeNull());
  });
});
