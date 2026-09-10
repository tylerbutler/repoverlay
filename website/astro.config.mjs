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
				"Add configuration files to git repositories without commits.",
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
							content: `repoverlay ${releaseVersion} is available.`,
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
					label: "Start here",
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
							label: "Quick start",
							slug: "quick-start",
						},
					],
				},
				{
					label: "Guides",
					items: [
						{
							label: "Applying overlays",
							slug: "guides/applying",
						},
						{
							label: "Creating and sharing",
							slug: "guides/creating",
						},
						{
							label: "Managing applied overlays",
							slug: "guides/managing",
						},
						{
							label: "The in-repo library",
							slug: "guides/library",
						},
					],
				},
				{
					label: "Advanced",
					items: [
						{
							label: "Restoring after git clean",
							slug: "guides/restoring",
						},
						{
							label: "Profiles",
							slug: "guides/profiles",
						},
						{
							label: "How it works",
							slug: "guides/how-it-works",
						},
						{
							label: "Migrating to 1.0",
							slug: "guides/migrating-to-1-0",
						},
					],
				},
				{
					label: "CLI reference",
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
