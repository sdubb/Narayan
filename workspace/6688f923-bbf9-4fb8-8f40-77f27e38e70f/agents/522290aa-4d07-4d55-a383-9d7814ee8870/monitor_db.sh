#!/bin/bash
# Simple DB health check
DB_CONN="<user_provided_connection_string>"
if ! echo "SELECT 1" | mysql "$DB_CONN" > /dev/null 2>&1; then
  echo "$(date): DB health check FAILED" >> ./db_monitor.log
  # send alert (email/notification placeholder)
  curl -X POST -H "Content-Type: application/json" -d '{"text":"Database health check failed"}' https://api.notification.service/alert
else
  echo "$(date): DB health check OK" >> ./db_monitor.log
fi
