import starlight from "@astrojs/starlight";
import starlightCatppuccin from "@catppuccin/starlight";
import a11yEmoji from "@fec/remark-a11y-emoji";
import { includeMarkdown } from "@hashicorp/platform-remark-plugins";
import { defineConfig } from "astro/config";
import { remarkShiftHeadings } from "remark-shift-headings";
import starlightLinksValidator from "starlight-links-validator";

// Get the directory name from the script URL
const rootDir = new URL(".", import.meta.url).pathname;

// https://astro.build/config
export default defineConfig({
	site: "https://repoverlay.tylerbutler.com",
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
			plugins: [starlightCatppuccin({
				dark: { flavor: "mocha", accent: "blue" },
				light: { flavor: "latte", accent: "blue" },
			}), starlightLinksValidator()],
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
		smartypants: false,
		remarkPlugins: [
			a11yEmoji,
			[includeMarkdown, { resolveMdx: true, resolveFrom: rootDir }],
			remarkShiftHeadings,
		],
	},
});
