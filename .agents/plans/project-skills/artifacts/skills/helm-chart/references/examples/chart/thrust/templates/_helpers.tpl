{{/* Names are capped at 63 chars (DNS label limit) and the release name leads. */}}
{{- define "thrust.name" -}}
{{- .Chart.Name | trunc 63 | trimSuffix "-" -}}
{{- end -}}

{{- define "thrust.fullname" -}}
{{- if contains .Chart.Name .Release.Name -}}
{{- .Release.Name | trunc 63 | trimSuffix "-" -}}
{{- else -}}
{{- printf "%s-%s" .Release.Name .Chart.Name | trunc 63 | trimSuffix "-" -}}
{{- end -}}
{{- end -}}

{{- define "thrust.labels" -}}
helm.sh/chart: {{ printf "%s-%s" .Chart.Name .Chart.Version | replace "+" "_" | trunc 63 | trimSuffix "-" }}
app.kubernetes.io/name: {{ include "thrust.name" . }}
app.kubernetes.io/instance: {{ .Release.Name }}
app.kubernetes.io/version: {{ .Chart.AppVersion | quote }}
app.kubernetes.io/managed-by: {{ .Release.Service }}
{{- end -}}

{{/*
Selector labels are name + instance. Every pod and selector in this chart ALSO carries a
`app.kubernetes.io/component` (server | migrate | postgresql) so no selector matches another
component's pods. A Deployment selector is immutable once created.
*/}}
{{- define "thrust.selectorLabels" -}}
app.kubernetes.io/name: {{ include "thrust.name" . }}
app.kubernetes.io/instance: {{ .Release.Name }}
{{- end -}}

{{/* digest wins over tag; empty tag falls back to appVersion */}}
{{- define "thrust.image" -}}
{{- if .Values.image.digest -}}
{{- printf "%s@%s" .Values.image.repository .Values.image.digest -}}
{{- else -}}
{{- printf "%s:%s" .Values.image.repository (default .Chart.AppVersion .Values.image.tag) -}}
{{- end -}}
{{- end -}}

{{- define "thrust.postgresImage" -}}
{{- $i := .Values.postgresql.image -}}
{{- if $i.digest -}}
{{- printf "%s:%s@%s" $i.repository $i.tag $i.digest -}}
{{- else -}}
{{- printf "%s:%s" $i.repository $i.tag -}}
{{- end -}}
{{- end -}}

{{/*
DATABASE_URL for containers. Credentials only ever arrive through secretKeyRef; the rendered
manifest never contains a password. With the dev Postgres, POSTGRES_PASSWORD must be listed BEFORE
DATABASE_URL because Kubernetes expands $(VAR) only from earlier entries of the same env list.
*/}}
{{- define "thrust.databaseEnv" -}}
{{- if .Values.postgresql.enabled -}}
- name: POSTGRES_PASSWORD
  valueFrom:
    secretKeyRef:
      name: {{ required "postgresql.auth.existingSecret is required when postgresql.enabled=true" .Values.postgresql.auth.existingSecret | quote }}
      key: postgres-password
- name: DATABASE_URL
  value: {{ printf "postgres://postgres:$(POSTGRES_PASSWORD)@%s-postgresql:5432/%s" (include "thrust.fullname" .) .Values.postgresql.auth.database | quote }}
{{- else -}}
- name: DATABASE_URL
  valueFrom:
    secretKeyRef:
      name: {{ required "database.existingSecret is required when postgresql.enabled=false" .Values.database.existingSecret | quote }}
      key: {{ .Values.database.existingSecretKey | quote }}
{{- end -}}
{{- end -}}
