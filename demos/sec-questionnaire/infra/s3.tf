# Object storage for customer data.

resource "aws_s3_bucket" "customer_uploads" {
  bucket = "tamsin-prod-customer-uploads"
}

resource "aws_s3_bucket_server_side_encryption_configuration" "customer_uploads" {
  bucket = aws_s3_bucket.customer_uploads.id
  rule {
    apply_server_side_encryption_by_default {
      sse_algorithm     = "aws:kms"
      kms_master_key_id = aws_kms_key.data.arn
    }
  }
}

resource "aws_s3_bucket" "audit_logs" {
  bucket = "tamsin-prod-audit-logs"
}

resource "aws_s3_bucket_server_side_encryption_configuration" "audit_logs" {
  bucket = aws_s3_bucket.audit_logs.id
  rule {
    apply_server_side_encryption_by_default {
      sse_algorithm = "AES256"
    }
  }
}

# Scheduled CSV exports of customer dashboards, picked up by customers'
# BI tools through pre-signed URLs. Added for the exports feature (PLAT-812).
resource "aws_s3_bucket" "analytics_exports" {
  bucket = "tamsin-prod-analytics-exports"
}

resource "aws_s3_bucket_lifecycle_configuration" "analytics_exports" {
  bucket = aws_s3_bucket.analytics_exports.id
  rule {
    id     = "expire-exports"
    status = "Enabled"
    expiration {
      days = 14
    }
  }
}

resource "aws_s3_bucket_public_access_block" "all" {
  for_each                = toset([aws_s3_bucket.customer_uploads.id, aws_s3_bucket.audit_logs.id, aws_s3_bucket.analytics_exports.id])
  bucket                  = each.value
  block_public_acls       = true
  block_public_policy     = true
  ignore_public_acls      = true
  restrict_public_buckets = true
}
