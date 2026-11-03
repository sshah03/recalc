resource "aws_backup_vault" "prod" {
  name        = "tamsin-prod-vault"
  kms_key_arn = aws_kms_key.data.arn
}

resource "aws_backup_plan" "daily" {
  name = "tamsin-prod-daily"

  rule {
    rule_name         = "daily-0300"
    target_vault_name = aws_backup_vault.prod.name
    schedule          = "cron(0 3 * * ? *)"
    lifecycle {
      delete_after = 35
    }
  }
}

resource "aws_backup_selection" "databases" {
  name         = "prod-databases"
  plan_id      = aws_backup_plan.daily.id
  iam_role_arn = "arn:aws:iam::000000000000:role/tamsin-backup"
  resources    = [aws_db_instance.primary.arn]
}
