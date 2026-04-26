function createConversationRuntimeModule() {
  const parseConversationIdFromUrl = () => {
    try {
      const url = new URL(window.location.href);
      const match = url.pathname.match(/\/c\/([^/?#]+)/i);
      return match?.[1] ?? null;
    } catch {
      return null;
    }
  };

  const currentPathname = () => {
    try {
      return new URL(window.location.href).pathname.toLowerCase();
    } catch {
      return "";
    }
  };

  return {
    parseConversationIdFromUrl,
    currentPathname,
  };
}
