<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>Label</key><string>{{LABEL}}</string>
<key>ProgramArguments</key><array><string>/bin/zsh</string><string>{{SERVICE_DIR}}/launchd-run.sh</string></array>
<key>WorkingDirectory</key><string>{{SERVICE_DIR}}</string>
<key>EnvironmentVariables</key><dict><key>CLOCKWORK_ENV_FILE</key><string>{{ENV_FILE}}</string><key>CLOCKWORK_STATE_DIR</key><string>{{STATE_DIR}}</string></dict>
<key>RunAtLoad</key><true/><key>KeepAlive</key><true/>
<key>StandardOutPath</key><string>{{STATE_DIR}}/logs/stdout.log</string>
<key>StandardErrorPath</key><string>{{STATE_DIR}}/logs/stderr.log</string>
</dict></plist>
