import { tDynamic, t } from "../i18n";

/** Friendly text for a `VoiceStatus.problem` i18n key; unknown keys get a gentle generic line. */
export function voiceProblemText(problem: string, name: string): string {
  const text = tDynamic(problem, { name });
  return text === problem ? t("voice.error.generic") : text;
}
