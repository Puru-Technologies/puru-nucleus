/**
 * Custom error types for puru-nucleus
 */

export type ErrorSeverity = 'warning' | 'error' | 'critical';

export class AppError extends Error {
  constructor(
    public code: string,
    public userMessage: string,
    public originalError?: unknown,
    public severity: ErrorSeverity = 'error'
  ) {
    super(userMessage);
    this.name = 'AppError';
  }

  static fromTauriError(error: string): AppError {
    // Parse severity from message patterns
    let severity: ErrorSeverity = 'error';

    if (error.includes('not running') || error.includes('Disk space')) {
      severity = 'critical';
    } else if (error.includes('not found') || error.includes('expired')) {
      severity = 'warning';
    }

    return new AppError('TAURI_ERROR', error, error, severity);
  }
}

// Specific error types
export class ValidationError extends AppError {
  constructor(message: string, public field?: string) {
    super('VALIDATION', message, null, 'warning');
    this.name = 'ValidationError';
  }
}

export class NetworkError extends AppError {
  constructor(message: string, originalError?: unknown) {
    super('NETWORK', message, originalError, 'error');
    this.name = 'NetworkError';
  }
}

export class PermissionError extends AppError {
  constructor(message: string) {
    super('PERMISSION', message, null, 'error');
    this.name = 'PermissionError';
  }
}

export class NotFoundError extends AppError {
  constructor(resource: string) {
    super('NOT_FOUND', `${resource} not found`, null, 'warning');
    this.name = 'NotFoundError';
  }
}

export class LicenseError extends AppError {
  constructor(message: string) {
    super('LICENSE', message, null, 'warning');
    this.name = 'LicenseError';
  }
}

export class DockerError extends AppError {
  constructor(message: string) {
    super('DOCKER', message, null, 'critical');
    this.name = 'DockerError';
  }
}

export class DatabaseError extends AppError {
  constructor(message: string, originalError?: unknown) {
    super('DATABASE', message, originalError, 'error');
    this.name = 'DatabaseError';
  }
}

export class BackupError extends AppError {
  constructor(message: string, originalError?: unknown) {
    super('BACKUP', message, originalError, 'error');
    this.name = 'BackupError';
  }
}

/**
 * Error codes for Tauri backend
 */
export const ErrorCodes = {
  // Validation
  VALIDATION_FAILED: 'VALIDATION_FAILED',

  // Network
  NETWORK_ERROR: 'NETWORK_ERROR',
  GCS_CONNECTION_FAILED: 'GCS_CONNECTION_FAILED',
  FIRESTORE_CONNECTION_FAILED: 'FIRESTORE_CONNECTION_FAILED',

  // Permission
  PERMISSION_DENIED: 'PERMISSION_DENIED',

  // System
  DOCKER_NOT_RUNNING: 'DOCKER_NOT_RUNNING',
  MYSQL_CONNECTION_FAILED: 'MYSQL_CONNECTION_FAILED',
  DISK_SPACE_LOW: 'DISK_SPACE_LOW',

  // License
  LICENSE_EXPIRED: 'LICENSE_EXPIRED',
  LICENSE_INVALID: 'LICENSE_INVALID',

  // Config
  INVALID_CONFIG: 'INVALID_CONFIG',
  CONFIG_NOT_FOUND: 'CONFIG_NOT_FOUND',

  // Internal
  INTERNAL_ERROR: 'INTERNAL_ERROR'
} as const;

export type ErrorCode = typeof ErrorCodes[keyof typeof ErrorCodes];
