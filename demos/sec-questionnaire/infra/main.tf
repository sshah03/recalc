# Tamsin Labs production environment.
# The region and account are supplied per deployment by the pipeline
# (see the deploy job); they are not set in this repository.

terraform {
  required_version = ">= 1.6"
  required_providers {
    aws = {
      source  = "hashicorp/aws"
      version = "~> 5.40"
    }
  }
  backend "s3" {}
}

variable "aws_region" {
  type        = string
  description = "Region for the production deployment, set by the pipeline."
}

variable "environment" {
  type    = string
  default = "production"
}

provider "aws" {
  region = var.aws_region
  default_tags {
    tags = {
      environment = var.environment
      owner       = "platform"
    }
  }
}

resource "aws_kms_key" "data" {
  description         = "Customer data encryption key"
  enable_key_rotation = true
}
