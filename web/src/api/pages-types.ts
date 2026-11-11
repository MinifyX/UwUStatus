// Shapes of the status page, incident, token and badge JSON.

export interface MonitorChoice {
  id: number;
  name: string;
  group: string;
}

export const ACCENTS = ['pink', 'violet', 'sky', 'mint', 'amber', 'coral'] as const;
export type Accent = (typeof ACCENTS)[number];

export interface PageMonitor {
  id: number;
  /** Shown name; '' = the monitor's own name. */
  name: string;
  show_latency: boolean;
}

export interface PageGroup {
  name: string;
  monitors: PageMonitor[];
}

export interface PageInput {
  slug: string;
  title: string;
  description: string;
  domains: string[];
  lang: 'de' | 'en';
  auto_lang: boolean;
  accent: Accent;
  nyu: boolean;
  timezone: string;
  groups: PageGroup[];
}

export interface StatusPage extends PageInput {
  id: number;
  has_logo: boolean;
  /** `/s/{slug}` */
  preview_url: string;
  created: number;
  updated: number;
}

export const INCIDENT_STATUSES = ['investigating', 'identified', 'monitoring', 'resolved'] as const;
export type IncidentStatus = (typeof INCIDENT_STATUSES)[number];
export const IMPACTS = ['down', 'degraded', 'none'] as const;
export type Impact = (typeof IMPACTS)[number];

export interface IncidentUpdate {
  id: number;
  at: number;
  status: IncidentStatus;
  /** '' = automatic text. */
  body: string;
  display_body: string;
}

export interface Incident {
  id: number;
  /** '' = automatic title. */
  title: string;
  display_title: string;
  auto: boolean;
  impact: Impact;
  status: IncidentStatus;
  started: number;
  resolved: number | null;
  monitors: { id: number; name: string }[];
  /** Oldest first. */
  updates: IncidentUpdate[];
}

export interface NewIncident {
  title: string;
  body: string;
  status: IncidentStatus;
  impact: Impact;
  monitors: number[];
}

export type Scope = 'read' | 'metrics';

export interface ApiToken {
  id: number;
  name: string;
  scopes: Scope[];
  created: number;
  last_used: number | null;
}

export interface Badge {
  public_id: string;
  badge_public: boolean;
  on_page: boolean;
  status_url: string;
  uptime_url: string;
  latency_url: string;
}
