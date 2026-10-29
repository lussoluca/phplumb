const repository = 'https://github.com/lussoluca/phplumb'

export const links = {
  repository,
  releases: `${repository}/releases/latest`,
  issues: `${repository}/issues`,
  benchmarks: `${repository}/blob/main/docs/benchmarks.md`,
  development: `${repository}/blob/main/docs/development.md`,
  zed: `${repository}/blob/main/editors/zed/README.md`,
  license: `${repository}/blob/main/LICENSE`,
}

// Files in `public/` are not prefixed with the base path automatically.
export function asset(path: string) {
  return `${process.env.NEXT_PUBLIC_BASE_PATH ?? ''}${path}`
}
