import { logGeneration } from "../stores/generationQueue";

/**
 * Convert any thrown value into a human-readable Japanese string.
 * Handles Error subclasses, plain strings, and falls back to JSON for objects.
 */
export function formatError(error: unknown): string {
  if (error instanceof Error) return error.message;
  if (typeof error === "string") return error;
  if (error === null || error === undefined) return "不明なエラーが発生しました。";
  try {
    return JSON.stringify(error);
  } catch {
    return String(error);
  }
}

/**
 * Centralized error reporting: formats the error, records it to the generation
 * log, and returns the formatted message so callers can render it inline.
 * Pass a `context` prefix (e.g. "プロジェクトの保存") to give the log viewer
 * enough context to identify where the failure happened.
 */
export function reportError(context: string, error: unknown): string {
  const message = formatError(error);
  logGeneration("error", `${context}: ${message}`);
  return message;
}
