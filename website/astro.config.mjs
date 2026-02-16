import netlify from "@astrojs/netlify";
import starlight from "@astrojs/starlight";
import a11yEmoji from "@fec/remark-a11y-emoji";
import { includeMarkdown } from "@hashicorp/platform-remark-plugins";
import { defineConfig } from "astro/config";
import starlightLinksValidator from "starlight-links-validator";

// Get the directory name from the script URL
const rootDir = new URL(".", import.meta.url).pathname;

// https://astro.build/config
export default defineConfig({
	output: "server",
	adapter: netlify({
		imageCDN: false,
	}),
	site: "https://repoverlay.tylerbutler.com",
	// Prevent zod from being externalized to avoid conflicts between
	// Astro's bundled zod v3 and user-installed zod v4
	// See: https://github.com/withastro/astro/issues/14117
	vite: {
		ssr: {
			noExternal: ["zod"],
		},
	},
	integrations: [
		starlight({
			title: "repoverlay",
			description:
				"Overlay config files into git repositories without committing them.",
			lastUpdated: true,
			customCss: [
				"@fontsource/metropolis/400.css",
				"@fontsource/metropolis/600.css",
				"./src/styles/custom.css",
			],
			plugins: [starlightLinksValidator()],
			social: [
				{
					icon: "github",
					label: "GitHub",
					href: "https://github.com/tylerbutler/repoverlay",
				},
			],
			sidebar: [
				{
					label: "Start Here",
					items: [
						{
							label: "What is repoverlay?",
							slug: "introduction",
						},
						{
							label: "Installation",
							slug: "installation",
						},
						{
							label: "Quick Start",
							slug: "quick-start",
						},
					],
				},
				{
					label: "Concepts",
					autogenerate: { directory: "concepts" },
				},
				{
					label: "Guides",
					autogenerate: { directory: "guides" },
				},
				{
					label: "CLI Reference",
					slug: "cli-reference",
				},
			],
		}),
	],
	markdown: {
		remarkPlugins: [
			a11yEmoji,
			[includeMarkdown, { resolveMdx: true, resolveFrom: rootDir }],
		],
	},
});
