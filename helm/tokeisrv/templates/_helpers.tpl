{{- define "tokeisrv.name" -}}
{{- default .Chart.Name .Values.nameOverride }}
{{- end -}}

{{- define "tokeisrv.fullname" -}}
{{- if .Values.fullnameOverride }}
{{- printf "%s" .Values.fullnameOverride }}
{{- else }}
{{- printf "%s-%s" (include "tokeisrv.name" .) .Release.Name }}
{{- end }}
{{- end -}}

{{- define "tokeisrv.labels" -}}
app.kubernetes.io/name: {{ include "tokeisrv.name" . }}
app.kubernetes.io/instance: {{ .Release.Name }}
{{- end -}}
