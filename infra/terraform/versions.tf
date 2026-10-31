terraform {
  required_version = ">= 1.7"

  required_providers {
    cloudflare = {
      source  = "cloudflare/cloudflare"
      version = "~> 5.0"
    }
  }

  # Private bucket, created once outside Terraform. CI passes the credentials as AWS_ACCESS_KEY_ID and
  # AWS_SECRET_ACCESS_KEY (the secrets TF_BACKEND_ACCESS_KEY_ID and TF_BACKEND_SECRET_ACCESS_KEY).
  backend "s3" {
    bucket = "gnoblin-tfstate"
    key    = "gnoblin/terraform.tfstate"
    region = "auto"

    endpoints = {
      s3 = "https://d6d18e6ddbe1f32180aa5f3bb96d4fc6.r2.cloudflarestorage.com"
    }

    skip_credentials_validation = true
    skip_region_validation      = true
    skip_requesting_account_id  = true
    skip_metadata_api_check     = true
    skip_s3_checksum            = true
    use_path_style              = true
  }
}

# The token comes from CLOUDFLARE_API_TOKEN.
provider "cloudflare" {}
