import { renderers } from './renderers.mjs';
import { s as serverEntrypointModule } from './chunks/_@astrojs-ssr-adapter_CvSoi7hX.mjs';
import { manifest } from './manifest_DqFH6iZq.mjs';
import { createExports } from '@astrojs/netlify/ssr-function.js';

const serverIslandMap = new Map();;

const _page0 = () => import('./pages/_image.astro.mjs');
const _page1 = () => import('./pages/404.astro.mjs');
const _page2 = () => import('./pages/_---slug_.astro.mjs');
const pageMap = new Map([
    ["node_modules/.pnpm/astro@5.17.2_@netlify+blobs@10.6.0_@types+node@25.2.3_jiti@2.6.1_rollup@4.57.1_typescript@5.9.3_yaml@2.8.2/node_modules/astro/dist/assets/endpoint/generic.js", _page0],
    ["node_modules/.pnpm/@astrojs+starlight@0.37.6_astro@5.17.2_@netlify+blobs@10.6.0_@types+node@25.2.3_jiti@2._6524e0a640d84bd3e3a13e95c0cbfb5c/node_modules/@astrojs/starlight/routes/static/404.astro", _page1],
    ["node_modules/.pnpm/@astrojs+starlight@0.37.6_astro@5.17.2_@netlify+blobs@10.6.0_@types+node@25.2.3_jiti@2._6524e0a640d84bd3e3a13e95c0cbfb5c/node_modules/@astrojs/starlight/routes/static/index.astro", _page2]
]);

const _manifest = Object.assign(manifest, {
    pageMap,
    serverIslandMap,
    renderers,
    actions: () => import('./noop-entrypoint.mjs'),
    middleware: () => import('./_astro-internal_middleware.mjs')
});
const _args = {
    "middlewareSecret": "42f583c0-9c53-40ca-b1ee-7030c0421dc3"
};
const _exports = createExports(_manifest, _args);
const __astrojsSsrVirtualEntry = _exports.default;
const _start = 'start';
if (Object.prototype.hasOwnProperty.call(serverEntrypointModule, _start)) {
	serverEntrypointModule[_start](_manifest, _args);
}

export { __astrojsSsrVirtualEntry as default, pageMap };
