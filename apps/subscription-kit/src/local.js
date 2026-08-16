// Local model detection — lets host UIs offer "use a model already on this
// machine" beside the subscription path, with zero typing.
//
// Ollama is the common case: it serves http://127.0.0.1:11434 and lists
// installed models at /api/tags (the models themselves live under
// ~/.ollama/models, but the HTTP API is the stable contract — never parse the
// blob store directly).

export async function detectOllama() {
  try {
    const ctl = new AbortController();
    const t = setTimeout(() => ctl.abort(), 1500);
    const r = await fetch("http://127.0.0.1:11434/api/tags", { signal: ctl.signal });
    clearTimeout(t);
    if (!r.ok) return { available: false, models: [] };
    const data = await r.json();
    return {
      available: true,
      baseUrl: "http://127.0.0.1:11434/v1",
      models: (data.models || []).map((m) => ({
        id: m.name,
        sizeBytes: m.size ?? null,
        family: m.details?.family ?? null,
        parameterSize: m.details?.parameter_size ?? null,
      })),
    };
  } catch {
    return { available: false, models: [] };
  }
}

export async function detectLocal() {
  const ollama = await detectOllama();
  return { ollama };
}
