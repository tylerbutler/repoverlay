import type { APIRoute } from "astro";
import { readFileSync } from "node:fs";
import { resolve } from "node:path";

function getVersion(): string {
	const cargoToml = readFileSync(
		resolve(process.cwd(), "../Cargo.toml"),
		"utf-8",
	);
	const match = cargoToml.match(/^version\s*=\s*"(.+?)"/m);
	const version = match?.[1];
	if (!version) {
		throw new Error("Could not find version in Cargo.toml");
	}
	return version;
}

export const GET: APIRoute = () => {
	const version = getVersion();
	const body = `repoverlay ${version} is out now!
See the release notes: https://github.com/tylerbutler/repoverlay/releases/latest
`;

	return new Response(body, {
		headers: { "Content-Type": "text/plain; charset=utf-8" },
	});
};
