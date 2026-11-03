resource "aws_db_instance" "primary" {
  identifier                = "tamsin-prod-primary"
  engine                    = "postgres"
  engine_version            = "16.3"
  instance_class            = "db.r6g.xlarge"
  allocated_storage         = 500
  multi_az                  = true
  storage_encrypted         = true
  kms_key_id                = aws_kms_key.data.arn
  backup_retention_period   = 35
  deletion_protection       = true
  final_snapshot_identifier = "tamsin-prod-primary-final"
}

resource "aws_db_instance" "reporting_replica" {
  identifier          = "tamsin-prod-reporting"
  replicate_source_db = aws_db_instance.primary.identifier
  instance_class      = "db.r6g.large"
  storage_encrypted   = true
  kms_key_id          = aws_kms_key.data.arn
}
