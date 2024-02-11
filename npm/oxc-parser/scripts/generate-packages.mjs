// Code copied from [Rome](https://github.com/rome/tools/blob/main/npm/rome/scripts/generate-packages.mjs)

import { resolve } from "node:path";
import { fileURLToPath } from "node:url";
import * as fs from "node:fs";

const OXC_ROOT = resolve(fileURLToPath(import.meta.url), "../..");
const PACKAGES_ROOT = resolve(OXC_ROOT, "..");
const BINARY_ROOT = resolve(OXC_ROOT, "../../napi/parser");
const MANIFEST_PATH = resolve(OXC_ROOT, "package.json");

console.log('OXC_ROOT', OXC_ROOT);
console.log('PACKAGES_ROOT', PACKAGES_ROOT);
console.log('BINARY_ROOT', BINARY_ROOT);
console.log('MANIFEST_PATH', MANIFEST_PATH);

const LIBC_MAPPING = {
  "gnu": "glibc",
  "musl": "musl",
}

const rootManifest = JSON.parse(
  fs.readFileSync(MANIFEST_PATH).toString("utf-8")
);

function package_name(target) {
  return `@oxc-parser/binding-${target}`
}
function generateNativePackage(target) {
  const binaryName = `parser.${target}.node`;

  const packageRoot = resolve(PACKAGES_ROOT, `oxc-parser-${target}`);
  const binarySource = resolve(BINARY_ROOT, binaryName);
  const binaryTarget = resolve(packageRoot, binaryName);

  // Remove the directory just in case it already exists (it's autogenerated
  // so there shouldn't be anything important there anyway)
  fs.rmSync(packageRoot, { recursive: true, force: true });

  // Create the package directory
  console.log(`Create directory ${packageRoot}`);
  fs.mkdirSync(packageRoot);

  // Generate the package.json manifest
  const { version, author, license, homepage, bugs, repository } = rootManifest;

  const triple = target.split("-");
  const platform = triple[0];
  const arch = triple[1];
  const libc = triple[2] && { libc: [LIBC_MAPPING[triple[2]]] }
  const manifest = {
    name: package_name(target),
    version,
    main: binaryName,
    license,
    author,
    bugs,
    homepage,
    repository,
    files: [binaryName],
    cpu: [arch],
    os: [platform],
    ...libc
  };

  const manifestPath = resolve(packageRoot, "package.json");
  console.log(`Create manifest ${manifestPath}`);
  fs.writeFileSync(manifestPath, JSON.stringify(manifest, null, 2));

  console.log(`Copy binary ${binaryTarget}`);
  fs.copyFileSync(binarySource, binaryTarget);
}

function writeManifest() {
  const packageRoot = resolve(PACKAGES_ROOT, 'oxc-parser');
  const manifestPath = resolve(packageRoot, "package.json");

  console.log('packageRoot', packageRoot);

  const manifestData = JSON.parse(
    fs.readFileSync(manifestPath).toString("utf-8")
  );

  const nativePackages = TARGETS.map((target) => [
    package_name(target),
    rootManifest.version,
  ]);

  manifestData["version"] = rootManifest.version;
  manifestData["optionalDependencies"] = Object.fromEntries(nativePackages);

  console.log('manifestPath', manifestPath);
  console.log('manifestData', manifestData);

  const content = JSON.stringify(manifestData, null, 2);
  fs.writeFileSync(manifestPath, content);

  let files = ["index.js", "index.d.ts"];
  for (const file of files) {
    fs.copyFileSync(resolve(BINARY_ROOT, file), resolve(packageRoot, file));
  }
}

// NOTE: Must update npm/oxc-parser/package.json
const TARGETS = [
  "win32-x64-msvc",
  "win32-arm64-msvc",
  "linux-x64-gnu",
  "linux-arm64-gnu",
  "linux-x64-musl",
  "linux-arm64-musl",
  "darwin-x64",
  "darwin-arm64",
];

for (const target of TARGETS) {
  generateNativePackage(target);
}

writeManifest();
