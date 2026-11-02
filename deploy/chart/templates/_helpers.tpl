{{/* An image from values, refused unless pinned by digest. Call with (list $ "name"). */}}
{{- define "cubebot.image" -}}
{{- $name := index . 1 -}}
{{- $image := index (index . 0).Values.images $name -}}
{{- if not (regexMatch "@sha256:[0-9a-f]{64}$" $image) -}}
{{- fail (printf "images.%s must be name@sha256:<digest>, got %q" $name $image) -}}
{{- end -}}
{{- $image -}}
{{- end -}}

{{- define "cubebot.labels" -}}
app.kubernetes.io/part-of: cubebot
app.kubernetes.io/instance: {{ .Release.Name }}
{{- end -}}

{{- define "cubebot.databaseSecret" -}}
{{- .Values.externalDatabase.existingSecret | default "database" -}}
{{- end -}}
