// Shapes of the server's JSON.

export interface Me {
  name: string;
  email: string | null;
  method: 'oidc' | 'local';
}

export interface Methods {
  oidc: boolean;
  local: boolean;
  local_totp: boolean;
}

export interface Settings {
  retention_raw_days: number;
  retention_hourly_days: number;
  retention_daily_days: number;
  default_interval_s: number;
}
