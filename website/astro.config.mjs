import { readFileSync } from "node:fs";
import starlight from "@astrojs/starlight";
import starlightAnnouncement from "starlight-announcement";
import starlightBlog from "starlight-blog";
import starlightHeadingBadges from "starlight-heading-badges";
import a11yEmoji from "@fec/remark-a11y-emoji";
import { includeMarkdown } from "@hashicorp/platform-remark-plugins";
import { defineConfig } from "astro/config";
import { remarkShiftHeadings } from "remark-shift-headings";
import starlightLinksValidator from "starlight-links-validator";
import starlightLlmsTxt from "starlight-llms-txt";

// Get the directory name from the script URL
const rootDir = new URL(".", import.meta.url).pathname;
const cargoToml = readFileSync(new URL("../Cargo.toml", import.meta.url), "utf8");
const releaseVersion = cargoToml.match(/^version\s*=\s*"([^"]+)"/m)?.[1];
if (!releaseVersion) {
	throw new Error("Could not read repoverlay version from Cargo.toml");
}
const releaseAnnouncementId = `v${releaseVersion.replaceAll(".", "-")}`;

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
				"@fontsource-variable/schibsted-grotesk",
				"@fontsource/commit-mono/400.css",
				"@fontsource/commit-mono/700.css",
				"./src/styles/fonts.css",
				"./src/styles/custom.css",
			],
			plugins: [
				starlightBlog({
					title: "Blog",
					authors: {
						tylerbutler: {
							name: "Tyler Butler",
							title: "Author of repoverlay",
							url: "https://github.com/tylerbutler",
						},
					},
				}),
				starlightAnnouncement({
					announcements: [
						{
							id: releaseAnnouncementId,
							content: `repoverlay ${releaseVersion} is out now.`,
							variant: "tip",
							dismissible: true,
							link: {
								text: "Release notes",
								href: `https://github.com/tylerbutler/repoverlay/releases/tag/v${releaseVersion}`,
							},
						},
					],
				}),
				starlightHeadingBadges(),
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
							label: "The In-Repo Library",
							slug: "guides/library",
						},
						{
							label: "Restoring After Git Clean",
							slug: "guides/restoring",
						},
						{
							label: "Profiles",
							slug: "guides/profiles",
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
