import type { ConfigSchemaProperty } from "../../types/core";

type SourceFieldProps = {
  fieldKey: string;
  property: ConfigSchemaProperty;
  value: unknown;
  required: boolean;
  secret: boolean;
  keepSecret: boolean;
  onChange: (value: unknown) => void;
  onKeepSecretChange: (keep: boolean) => void;
};

export function SourceField({
  fieldKey,
  property,
  value,
  required,
  secret,
  keepSecret,
  onChange,
  onKeepSecretChange,
}: SourceFieldProps) {
  const label = property.title ?? fieldKey;
  const enumValues = property.enum_values ?? [];

  if (enumValues.length > 0) {
    const selectedValue = value === undefined || value === null ? "" : JSON.stringify(value);
    return (
      <label className="text-xs text-[var(--muted-text)]">
        <span className="text-[var(--app-text)]">
          {label}
          {required ? " *" : ""}
        </span>
        <select
          className="ui-select ui-focus mt-1"
          value={selectedValue}
          onChange={(event) => {
            if (!event.currentTarget.value) {
              onChange("");
              return;
            }
            onChange(JSON.parse(event.currentTarget.value));
          }}
        >
          <option value="">请选择</option>
          {enumValues.map((item) => {
            const optionValue = JSON.stringify(item);
            return (
              <option key={optionValue} value={optionValue}>
                {String(item)}
              </option>
            );
          })}
        </select>
        <FieldHint property={property} />
      </label>
    );
  }

  if (property.property_type === "boolean") {
    return (
      <label className="ui-focus flex items-center gap-2 rounded-md border border-[var(--panel-border)] bg-[var(--panel-bg)] px-3 py-2 text-sm text-[var(--app-text)]">
        <input
          className="ui-focus"
          type="checkbox"
          checked={Boolean(value)}
          onChange={(event) => onChange(event.currentTarget.checked)}
        />
        <span>
          {label}
          {required ? " *" : ""}
        </span>
      </label>
    );
  }

  if (property.property_type === "number" || property.property_type === "integer") {
    return (
      <label className="text-xs text-[var(--muted-text)]">
        <span className="text-[var(--app-text)]">
          {label}
          {required ? " *" : ""}
        </span>
        <input
          className="ui-input ui-focus mt-1"
          type="number"
          step={property.property_type === "integer" ? "1" : "any"}
          min={property.minimum}
          max={property.maximum}
          placeholder={property.x_ui?.placeholder}
          value={value === undefined || value === null ? "" : String(value)}
          onChange={(event) => {
            const raw = event.currentTarget.value;
            onChange(raw === "" ? "" : Number(raw));
          }}
        />
        <FieldHint property={property} />
      </label>
    );
  }

  const isPassword = secret || property.format === "password";
  return (
    <label className="text-xs text-[var(--muted-text)]">
      <span className="text-[var(--app-text)]">
        {label}
        {required ? " *" : ""}
      </span>
      <input
        className="ui-input ui-focus mt-1"
        type={isPassword ? "password" : "text"}
        minLength={property.min_length}
        maxLength={property.max_length}
        pattern={property.pattern}
        placeholder={
          isPassword && keepSecret
            ? "留空保持现有密钥"
            : property.x_ui?.placeholder ?? property.description
        }
        value={typeof value === "string" ? value : value === undefined ? "" : String(value)}
        onChange={(event) => {
          const next = event.currentTarget.value;
          onChange(next);
          if (isPassword) {
            onKeepSecretChange(next.length === 0);
          }
        }}
      />
      <FieldHint property={property} />
    </label>
  );
}

function FieldHint({ property }: { property: ConfigSchemaProperty }) {
  if (!property.x_ui?.help && !property.description) {
    return null;
  }
  return (
    <p className="mt-1 text-[11px] text-[var(--muted-text)]">
      {property.x_ui?.help ?? property.description}
    </p>
  );
}
