// The Worker fixture exercises the plugin with a minimal host contract. The
// full EmDash package requires Astro virtual modules outside this test Worker.
export const definePlugin = (descriptor) => descriptor;
export class PluginRouteError extends Error {}
