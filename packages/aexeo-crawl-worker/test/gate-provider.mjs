export default {
  async fetch(request) {
    const { input, preset, collection } = await request.json();
    if (preset !== "custom" || collection !== "pages") return new Response(null, { status: 400 });
    const resultInput = input.id === "/stale" ? { ...input, id: "/old" } : input;
    return Response.json([{ extension: "example.rule", parameters: { mode: "strict" }, input: resultInput,
      diagnostics: [{ message: "Review this opening", line: 1, column: 1 }] }]);
  },
};
