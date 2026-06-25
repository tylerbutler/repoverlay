import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

const page = readFileSync(new URL("./index.astro", import.meta.url), "utf8");

test("homepage explains the overlay mechanism instead of a generic feature bento", () => {
	assert.match(page, /const mechanism = \[/);
	assert.match(page, /class="mechanism"/);
	assert.match(page, /Source/);
	assert.match(page, /\.git\/info\/exclude/);
	assert.match(page, /restore/);
	assert.match(page, /repoverlay browse tylerbutler/);
	assert.doesNotMatch(page, /const steps = \[/);
	assert.doesNotMatch(page, /class="flow/);
	assert.doesNotMatch(page, /const features = \[/);
	assert.doesNotMatch(page, /class="bento"/);
});

test("homepage surfaces install commands before users leave the hero", () => {
	assert.match(page, /const installs = \[/);
	assert.match(page, /brew install tylerbutler\/tap\/repoverlay/);
	assert.match(page, /cargo binstall repoverlay/);
	assert.match(page, /class="install-strip"/);
});

test("copy buttons provide assistive status and failure feedback", () => {
	assert.match(page, /aria-live="polite"/);
	assert.match(
		page,
		/<a class="skip" href="#main">Skip to content<\/a>\n\s+<span\n\s+class="copy-status"\n\s+data-copy-status/,
	);
	assert.match(page, /aria-label=\{`Copy \$\{install\.label\} install command`\}/);
	assert.match(page, /label: "Homebrew"/);
	assert.match(page, /label: "Cargo"/);
	assert.match(page, /aria-label="Copy example apply command"/);
	assert.match(page, /data-copy-status/);
	assert.match(page, /Copy failed/);
	assert.match(page, /const copyTimers = new WeakMap/);
	assert.match(page, /let statusTimer:/);
	assert.match(page, /const setCopyStatus = /);
	assert.match(page, /window\.clearTimeout/);
	assert.equal(page.match(/data-copy-status/g)?.length, 2);
	assert.doesNotMatch(page, /\/\* clipboard unavailable \*\//);
});

test("copy controls adapt to touch input", () => {
	assert.match(page, /min-height:\s*44px/);
	assert.match(page, /@media \(pointer: coarse\)/);
});

test("mobile navigation keeps the CLI reference available", () => {
	assert.doesNotMatch(page, /a\[href="\/cli-reference\/"\]\s*\{\s*display:\s*none;/);
});

test("homepage copy avoids em-dash cadence", () => {
	assert.doesNotMatch(page, /—/);
});
