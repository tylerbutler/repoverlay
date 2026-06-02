import starlight from "@astrojs/starlight";
import starlightCatppuccin from "@catppuccin/starlight";
import a11yEmoji from "@fec/remark-a11y-emoji";
import { includeMarkdown } from "@hashicorp/platform-remark-plugins";
import { defineConfig } from "astro/config";
import { remarkShiftHeadings } from "remark-shift-headings";
import starlightLinksValidator from "starlight-links-validator";
import starlightLlmsTxt from "starlight-llms-txt";

// Get the directory name from the script URL
const rootDir = new URL(".", import.meta.url).pathname;

// https://astro.build/config
export default defineConfig({
	site: "https://repoverlay.tylerbutler.com",
	prefetch: {
		defaultStrategy: "hover",
		prefetchAll: true,
	},
	integrations: [
		starlight({
			title: "repoverlay",
			editLink: {
				baseUrl:
					"https://github.com/tylerbutler/repoverlay/edit/main/website/",
			},
			logo: {
				src: "./src/assets/repoverlay.svg",
			},
			favicon: "./src/assets/repoverlay.svg",
			description:
				"Overlay config files into git repositories without committing them.",
			lastUpdated: true,
			customCss: [
				"@fontsource/metropolis/400.css",
				"@fontsource/metropolis/600.css",
				"./src/styles/fonts.css",
				"./src/styles/custom.css",
			],
			plugins: [
				// starlightCatppuccin({
				// 	dark: { flavor: "macchiato", accent: "maroon" },
				// 	light: { accent: "maroon" },
				// }),
				starlightLlmsTxt(),
				starlightLinksValidator(),
			],
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
					label: "Guides",
					items: [
						{
							label: "Applying Overlays",
							slug: "guides/applying",
						},
						{
							label: "Creating & Sharing",
							slug: "guides/creating",
						},
						{
							label: "Managing Applied Overlays",
							slug: "guides/managing",
						},
						{
							label: "Restoring After Git Clean",
							slug: "guides/restoring",
						},
						{
							label: "How It Works",
							slug: "guides/how-it-works",
						},
						{
							label: "Migrating to 1.0",
							slug: "guides/migrating-to-1-0",
						},
					],
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
