import { makePolicy, type RepopoConfig } from "repopo";
import {
  LicenseFileExists,
  NoLargeBinaryFiles,
  RequiredGitignorePatterns,
} from "repopo/policies";

const config: RepopoConfig = {
  excludeFiles: ["node_modules/", "target/", "docs/talks/node_modules/"],

  policies: [
    makePolicy(LicenseFileExists),
    makePolicy(NoLargeBinaryFiles),
    makePolicy(RequiredGitignorePatterns, {
      patterns: [
        { pattern: "/target/", comment: "Rust build output" },
        { pattern: "/coverage/", comment: "Coverage reports" },
      ],
    }),
  ],
};

export default config;
