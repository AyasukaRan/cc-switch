import { useState } from "react";
import { useTranslation } from "react-i18next";
import { SettingsRow } from "./SettingsLayout";
import { Button } from "@/components/ui/button";
import { Textarea } from "@/components/ui/textarea";
import { getCodexCustomTemplate } from "@/config/codexTemplates";
import { useCodexTomlValidation } from "@/components/providers/forms/hooks/useCodexTomlValidation";

interface Props {
  template?: string;
  onSave: (template: string | undefined) => Promise<boolean>;
}

export function CodexTemplateSettings({ template, onSave }: Props) {
  const { t } = useTranslation();
  const [draft, setDraft] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  const { configError, validateToml, clearError } = useCodexTomlValidation();
  const title = t("settings.codexTemplate.title");

  async function save(value: string | undefined) {
    if (value !== undefined && !validateToml(value)) return;
    setSaving(true);
    try {
      if (await onSave(value)) setDraft(null);
    } finally {
      setSaving(false);
    }
  }

  return (
    <SettingsRow
      label={title}
      htmlFor={draft !== null ? "codex-template" : undefined}
      help={{ title, body: t("settings.codexTemplate.help") }}
      control={
        draft === null && (
          <Button
            type="button"
            size="compact"
            variant="neutral"
            onClick={() => {
              clearError();
              setDraft(template ?? getCodexCustomTemplate().config);
            }}
          >
            {t("common.edit")}
          </Button>
        )
      }
    >
      {draft !== null && (
        <div className="space-y-2">
          <Textarea
            id="codex-template"
            className="font-mono text-sm"
            rows={9}
            value={draft}
            disabled={saving}
            aria-invalid={!!configError}
            aria-describedby={configError ? "codex-template-error" : undefined}
            onChange={(event) => {
              setDraft(event.target.value);
              clearError();
            }}
          />
          {configError && (
            <p
              id="codex-template-error"
              role="alert"
              className="whitespace-pre-wrap text-xs text-danger-text"
            >
              {configError}
            </p>
          )}
          <div className="flex flex-wrap items-center justify-end gap-2">
            {template !== undefined && (
              <Button
                type="button"
                size="compact"
                variant="quiet"
                className="me-auto"
                disabled={saving}
                onClick={() => void save(undefined)}
              >
                {t("settings.codexTemplate.restore")}
              </Button>
            )}
            <Button
              type="button"
              size="compact"
              variant="neutral"
              disabled={saving}
              onClick={() => setDraft(null)}
            >
              {t("common.cancel")}
            </Button>
            <Button
              type="button"
              size="compact"
              variant="solid"
              disabled={saving}
              onClick={() => void save(draft)}
            >
              {t("common.save")}
            </Button>
          </div>
        </div>
      )}
    </SettingsRow>
  );
}
