# Infrastructure

What runs outside this repository, and where it is defined. New infrastructure code goes in this folder. CI workflows stay in `.github/workflows/`, because GitHub requires that path.

Everything is on one Cloudflare account, zone `gnoblin.org`.

| Part                     | What it is                                                                      | Defined in                                                                                     |
| ------------------------ | ------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------- |
| Website and docs         | Worker `gnoblin-website` (static assets) on `gnoblin.org` and `www.gnoblin.org` | `infra/terraform/`, `infra/website/`, `.github/workflows/infra.yml`                            |
| Package repositories     | R2 bucket `gnoblin-repo`, served at `repo.gnoblin.org`                          | `scripts/pkgrepo`, `.github/workflows/package-repository.yml`, `.github/workflows/nightly.yml` |
| Repository index (pages) | Worker `repo-index` on `repo.gnoblin.org/*`                                     | `infra/terraform/`, `infra/repo-index/`                                                        |

## Package repositories

- **Bucket:** `gnoblin-repo`, public through the custom domain `repo.gnoblin.org` (minimum TLS 1.2). Gnoblin and Bingux share it. The publish step adds `<project>/<channel>` to the path, so they do not overwrite each other.
- **Signing key:** one RSA 4096 key, no expiry. The fingerprint is in `docs/package-repositories.md` and the public key is at `https://repo.gnoblin.org/gpg.key`. The private key and passphrase are the CI secrets `REPO_GPG_PRIVATE_KEY` and `REPO_GPG_PASSPHRASE`, set in both repositories.
- **Upload token:** an R2 API token named `gnoblin-repo-ci`, Object Read and Write on `gnoblin-repo` only. It is the CI secrets `AWS_ACCESS_KEY_ID` and `AWS_SECRET_ACCESS_KEY`. The CI variables are `REPO_BACKEND=s3`, `REPO_DESTINATION=s3://gnoblin-repo` and `AWS_ENDPOINT_URL` (the account's R2 endpoint).
- **Rotating the token:** create a new token with the same scope, replace the two secrets in both repositories, then delete the old token.

## Repository index

R2 serves exact object names only, so a folder URL has no listing. The Worker in `infra/repo-index/` answers paths that end in `/` with an HTML list of that folder. All other paths go straight to the bucket, so `dnf`, `apt` and `pacman` are not affected.

Terraform deploys it (see Terraform below). To build the bundle locally:

```sh
cd infra/repo-index
yarn install
yarn build
```

Every request to `repo.gnoblin.org` runs the Worker, including package downloads. The free plan has a daily request limit.

Do not add a zone redirect rule for `repo.gnoblin.org`. Redirect rules run before Workers and would hide the index.

## Terraform

`infra/terraform/` defines the R2 buckets, the mail DNS records and both Workers. State is in the private R2 bucket `gnoblin-tfstate`. The R2 custom domain `repo.gnoblin.org` is not in Terraform, because the provider cannot import it.

`.github/workflows/infra.yml` builds the Worker bundle and the website, then runs `terraform plan`. Pull requests only check formatting and validity, because the secrets in the `infra` environment are limited to `main`. A run on `main` applies the plan only when the repository variable `INFRA_AUTO_APPLY` is `true`. A manual run applies only when you set the `apply` input.

The website Worker needs `gnoblin.org` and `www.gnoblin.org` to have no existing DNS record. The old CNAMEs to `gnoblin-docs.pages.dev` must be deleted before the first apply. Between that delete and the apply the site is down.
