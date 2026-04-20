import { createHash } from "node:crypto";
import type { Page } from "playwright-core";

function normalizeOptionalString(value: string | null | undefined): string | null {
  const trimmed = value?.trim();
  return trimmed ? trimmed : null;
}

function safeDecodeURIComponent(value: string): string {
  try {
    return decodeURIComponent(value);
  } catch {
    return value;
  }
}

function pathnameSegments(pathname: string): string[] {
  return pathname.split("/").filter(Boolean);
}

function pathnameFromSegments(segments: string[]): string {
  return segments.length > 0 ? `/${segments.join("/")}` : "/";
}

function conversationWorkspaceSegments(pathname: string): string[] {
  const segments = pathnameSegments(pathname);
  if (
    segments.length >= 2 &&
    segments[segments.length - 2]?.toLowerCase() === "c"
  ) {
    return segments.slice(0, -2);
  }
  return segments;
}

function conversationWorkspacePathname(pathname: string): string {
  return pathnameFromSegments(conversationWorkspaceSegments(pathname));
}

function conversationPathname(
  workspacePathname: string,
  conversationId: string,
): string {
  const segments = conversationWorkspaceSegments(workspacePathname);
  segments.push("c", encodeURIComponent(conversationId));
  return pathnameFromSegments(segments);
}

export function deepResearchPathname(pathname: string): string {
  const segments = conversationWorkspaceSegments(pathname);
  if (segments[segments.length - 1]?.toLowerCase() === "deep-research") {
    return pathnameFromSegments(segments);
  }
  segments.push("deep-research");
  return pathnameFromSegments(segments);
}

function conversationIdFromPathname(pathname: string): string | null {
  const segments = pathnameSegments(pathname);
  if (
    segments.length >= 2 &&
    segments[segments.length - 2]?.toLowerCase() === "c"
  ) {
    return normalizeOptionalString(
      safeDecodeURIComponent(segments[segments.length - 1] ?? ""),
    );
  }
  return null;
}

export function conversationIdFromUrl(
  rawUrl: string | null | undefined,
): string | null {
  const resolvedUrl = normalizeOptionalString(rawUrl);
  if (!resolvedUrl) {
    return null;
  }
  try {
    const url = new URL(resolvedUrl);
    return conversationIdFromPathname(url.pathname);
  } catch {
    return null;
  }
}

export function resolveConversationUrl(
  rawUrl: string,
  conversationId?: string | null,
): string {
  try {
    const url = new URL(rawUrl);
    url.pathname = conversationWorkspacePathname(url.pathname);
    const normalizedConversationId = normalizeOptionalString(conversationId);
    if (normalizedConversationId) {
      url.pathname = conversationPathname(
        url.pathname,
        normalizedConversationId,
      );
    }
    return url.toString();
  } catch {
    return rawUrl;
  }
}

export function workspaceKeyFromUrl(rawUrl: string): string {
  try {
    const url = new URL(rawUrl);
    const params = new URLSearchParams(url.search);
    params.delete("scenario");
    const query = params.toString();
    const pathname = conversationWorkspacePathname(url.pathname);
    return `${url.origin}${pathname}${query ? `?${query}` : ""}`;
  } catch {
    return rawUrl;
  }
}

export function matchesTargetWorkspace(pageUrl: string, targetUrl: string): boolean {
  try {
    return workspaceKeyFromUrl(pageUrl) === workspaceKeyFromUrl(targetUrl);
  } catch {
    return false;
  }
}

export function matchesTargetLocation(pageUrl: string, targetUrl: string): boolean {
  const targetConversationId = conversationIdFromUrl(targetUrl);
  if (targetConversationId) {
    return conversationIdFromUrl(pageUrl) === targetConversationId;
  }
  return matchesTargetWorkspace(pageUrl, targetUrl);
}

export function researchRecoveryLocationScore(
  pageUrl: string | null,
  targetUrl: string,
  expectedConversationId: string | null,
): number {
  if (!pageUrl) {
    return -1;
  }
  const pageConversationId = conversationIdFromUrl(pageUrl);
  if (expectedConversationId) {
    if (pageConversationId === expectedConversationId) {
      return 100;
    }
    if (
      matchesTargetWorkspace(
        pageUrl,
        resolveConversationUrl(targetUrl, expectedConversationId),
      )
    ) {
      return 10;
    }
    return -1;
  }
  return matchesTargetWorkspace(pageUrl, targetUrl) ? 0 : -1;
}

export function uniquePages(pages: Page[]): Page[] {
  return pages.filter((page, index, all) => all.indexOf(page) === index);
}

export function buildTargetPageId(page: Page): string {
  return createHash("sha1")
    .update(
      `${workspaceKeyFromUrl(page.url())}:${page.context().pages().indexOf(page)}`,
    )
    .digest("hex")
    .slice(0, 16);
}
