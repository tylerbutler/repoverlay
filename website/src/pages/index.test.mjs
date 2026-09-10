import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

const page = readFileSync(new URL("./index.astro", import.meta.url), "utf8");
const quickStart = readFileSync(new URL("../content/docs/quick-start.mdx", import.meta.url), "utf8");

// These tests assert the homepage's observable contract: which sections exist,
// what commands are surfaced, and which accessibility/UX guarantees hold. They
// deliberately avoid pinning internal implementation details (variable names,
// timer data structures, exact node counts) so the page can be refactored
// without churning the suite.

test("homepage centers the overlay mechanism, not a generic feature grid", () => {
	assert.match(page, /class="mechanism[ "]/);
	// The lifecycle is spelled out: a named source, the git guard, and recovery.
	assert.match(page, /Source/);
	assert.match(page, /\.git\/info\/exclude/);
	assert.match(page, /\brestore\b/i);
	// A real browse example pointed at a concrete source.
	assert.match(page, /repoverlay browse tylerbutler/);
	// No generic bento / feature-card grid.
	assert.doesNotMatch(page, /class="bento"/);
	assert.doesNotMatch(page, /\bcell--/);
});

test("install commands precede supporting explanations", () => {
	assert.match(page, /class="install-strip[ "]/);
	assert.match(page, /brew install tylerbutler\/tap\/repoverlay/);
	assert.match(page, /cargo install repoverlay/);
	// The install strip renders before the deeper explanatory sections.
	const installIdx = page.indexOf('class="install-strip');
	const mechanismIdx = page.indexOf('class="mechanism');
	assert.ok(installIdx > -1, "install-strip is present");
	assert.ok(mechanismIdx > -1, "mechanism is present");
	assert.ok(installIdx < mechanismIdx, "install strip precedes the mechanism");
	assert.ok(installIdx < page.indexOf('class="problem'), "install strip precedes the problem");
	assert.match(page, /href="\/installation\/"/);
	assert.match(page, /class="btn btn--primary" href="\/quick-start\/"/);
});

test("commands are copyable with accessible names and failure feedback", () => {
	// A polite assistive status region exists for copy announcements.
	assert.match(page, /aria-live="polite"/);
	assert.match(page, /data-copy-status/);
	// Copy controls name the specific command they copy. (CopyButton renders the
	// label prop as the button's aria-label.)
	assert.match(page, /Copy \$\{[^}]+\} install command/);
	assert.match(page, /label="Copy example apply command"/);
	// Users are told when copying fails.
	assert.match(page, /Copy failed/);
	assert.match(page, /data-copy-notice/);
	assert.match(page, /data-dismiss-copy/);
});

test("interactive controls meet touch-target sizing", () => {
	assert.match(page, /min-height:\s*44px/);
	assert.match(page, /@media \(pointer: coarse\)/);
});

test("primary navigation keeps the CLI reference on mobile", () => {
	assert.doesNotMatch(
		page,
		/a\[href="\/cli-reference\/"\]\s*\{\s*display:\s*none/,
	);
});

test("the homepage and quick start share a dependency-free profile lifecycle", () => {
	for (const content of [page, quickStart]) {
		assert.match(content, /repoverlay profile apply rust-dev --harness claude/);
		assert.match(content, /repoverlay profile remove rust-dev --harness claude/);
		assert.doesNotMatch(content, /repoverlay remove rust-dev/);
	}
	const config = quickStart.match(/```ccl\n([\s\S]*?)```/)?.[1];
	assert.ok(config, "Quick Start defines the example profile");
	assert.match(config, /rust-dev\s*=/);
	assert.match(config, /content\s*=\s*\n\s*Be concise in all responses\./);
	assert.doesNotMatch(config, /overlays\s*=|plugins\s*=/);
	assert.doesNotMatch(page, /4 capabilities|cargo-review agent|crates\.io MCP server/);
	assert.match(page, /existing tracked/);
	assert.match(quickStart, /already tracked/);
});

test("marketing copy avoids em-dash cadence", () => {
	assert.doesNotMatch(page, /—/);
});
