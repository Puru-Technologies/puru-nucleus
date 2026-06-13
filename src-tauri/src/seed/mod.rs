//! First-run seeding — populate service databases, RabbitMQ queues, and
//! Jasper report templates so a fresh native deployment can boot and run.
//!
//! Catalogs mirror the puru-has one-time setup code (HASOneTimeController,
//! V5/V7 migrations) and the puru_config schema in puru_auth. Everything is
//! idempotent: existing values are never overwritten, so hospital-specific
//! edits made later in the UI survive re-runs.

use crate::config::{self, NucleusConfig};
use crate::error::NucleusError;
use mysql_async::prelude::*;
use serde::Serialize;
use std::collections::HashMap;

/// GCS prefix under which report templates live, laid out exactly as they
/// should appear on disk relative to puru_data_path. e.g. the object
/// `config-data/templates/opd/Invoice_Main.jrxml` lands at
/// `{puru_data}/config-data/templates/opd/Invoice_Main.jrxml`.
const TEMPLATES_GCS_PREFIX: &str = "config-data/templates/";

#[derive(Debug, Clone, Serialize)]
pub struct SeedSection {
    pub name: String,
    pub created: u64,
    pub skipped: u64,
    pub errors: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct SeedReport {
    pub sections: Vec<SeedSection>,
}

// ── puru_config defaults (puru_auth.puru_config) ─────────────────────────────
// Only rows whose config_value is still NULL/empty are filled in.

fn config_defaults(config: &NucleusConfig) -> Vec<(&'static str, String)> {
    let data_root = config
        .puru_data_path
        .as_deref()
        .unwrap_or("")
        .replace('\\', "/");
    let jwt_secret = generate_secret("jwt");
    let service_key = generate_secret("service-key");

    vec![
        ("jwt.secret", jwt_secret),
        ("jwtExpirationInMs", "86400000".into()),
        ("jwt.department.validity", "2592000".into()),
        ("jwt.staff.validity", "1800".into()),
        ("department.auth.default.pin", "1234".into()),
        ("puru.ins.name", config.hospital_code.clone()),
        ("puru.prod", "0".into()),
        ("env", "dev".into()),
        ("puru.server.ip", config.server_ip.clone()),
        ("puru.data.root.dir", data_root.clone()),
        ("puru.service.key", service_key),
        (
            "puru.hospital.name.en",
            format!("{} Hospital", config.hospital_code),
        ),
        ("puru.hospital.line2.en", "".into()),
        ("puru.hospital.line3.en", "".into()),
        ("puru.hospital.reg.no", "REG-001".into()),
        ("hospital.short.code", config.hospital_code.clone()),
        ("hospital.logo.url", "".into()),
        ("hospital.country-code", "91".into()),
        ("hospital.phone-length", "10".into()),
        ("barcode.prefix.store", "ST".into()),
        ("barcode.prefix.ppin", "PP".into()),
        ("barcode.prefix.pathology", "PL".into()),
        ("barcode.prefix.patient", "PA".into()),
        ("barcode.prefix.employee", "EM".into()),
        ("barcode.prefix.department", "DP".into()),
        ("barcode.prefix.medicalSaleInvoice", "MS".into()),
        ("barcode.prefix.medicalToken", "MT".into()),
        ("barcode.prefix.opd", "OP".into()),
        ("barcode.prefix.ft", "FT".into()),
        ("barcode.prefix.ipd", "IP".into()),
        ("barcode.prefix.medicalInvoiceSale", "MI".into()),
        ("barcode.prefix.medicalInvoiceReturn", "MR".into()),
        ("port.auth", "8080".into()),
        ("port.has", "8082".into()),
        ("port.xenon", "8081".into()),
        ("port.pacs", "8083".into()),
        ("port.path", "8084".into()),
        ("port.neon", "8087".into()),
        ("port.hr", "8089".into()),
        ("module.radiology", "true".into()),
        ("module.pathology", "true".into()),
        ("module.inventory", "true".into()),
        ("module.ipd", "true".into()),
        ("module.hr", "false".into()),
        ("module.attendance", "false".into()),
        ("module.frontoffice", "true".into()),
        ("module.panchkarma", "false".into()),
        ("module.ayurved", "false".into()),
        ("feature.onlyRadiology", "false".into()),
        ("feature.waiverText", "".into()),
        ("feature.fetchPrescription", "false".into()),
        ("feature.additionalDiscountPharmacy", "false".into()),
        ("feature.additionalDiscountPerc", "0".into()),
        ("feature.allowSelfAsConsultant", "true".into()),
        ("feature.zipUploadMandatory", "false".into()),
        ("feature.onlyOffline", "false".into()),
        ("spring.rabbitmq.enabled", "true".into()),
        ("spring.rabbitmq.host", "127.0.0.1".into()),
        ("spring.rabbitmq.port", "5672".into()),
        ("spring.rabbitmq.username", "puru".into()),
        ("spring.rabbitmq.password", "puru123".into()),
        ("spring.rabbitmq.listener.simple.retry.enabled", "true".into()),
        ("spring.rabbitmq.listener.simple.concurrency", "2".into()),
        ("spring.rabbitmq.listener.simple.prefetch", "10".into()),
        ("spring.rabbitmq.listener.simple.max-concurrency", "5".into()),
        ("spring.mail.host", "".into()),
        ("spring.mail.port", "587".into()),
        ("spring.mail.username", "".into()),
        ("spring.mail.password", "".into()),
        ("spring.mail.properties.mail.smtp.auth", "false".into()),
        ("spring.mail.properties.mail.smtp.starttls.enable", "false".into()),
        ("spring.mail.properties.mail.smtp.ssl.trust", "".into()),
        (
            "service.pacs.storagePath",
            format!("{}/pacs", data_root.trim_end_matches('/')),
        ),
        ("service.pacs.aet", "PURUPACS".into()),
        ("service.neon.gstEnabled", "true".into()),
        ("service.neon.lowStockThreshold", "10".into()),
    ]
}

// ── ref_data catalog (puru_has.ref_data) ─────────────────────────────────────
// (type, string_key, string_value, int_value, user_editable)
// Mirrors HASOneTimeController insertRefData* methods.

const REF_DATA: &[(&str, &str, &str, i32, bool)] = &[
    // Property — REQUIRED at boot: VariableService ctor NPEs when missing
    ("Property", "hospital.name.hi", "", 0, true),
    ("Property", "hospital.line2.hi", "", 0, true),
    // DrugFrequency
    ("DrugFrequency", "Before Meal", "भोजन के पहले", 2, false),
    ("DrugFrequency", "After Meal", "भोजन के बाद", 2, false),
    ("DrugFrequency", "Empty Stomach Morning", "सुबह खाली पेट", 1, false),
    ("DrugFrequency", "Before Sleeping", "रात सोते समय", 1, false),
    ("DrugFrequency", "2 Hours After Meal", "भोजन के 2 घंटे बाद", 2, false),
    // SCLASS — one row per service type (the RefData entity enforces
    // UNIQUE(type, stringKey)). The UI consumes these via ServiceController,
    // which joins string_values with ';' — so the full class catalog lives
    // in a single semicolon-separated value per type.
    ("SCLASS", "DOC_CHARGE", "Visit;Repeat Visit;Ward Visit", 0, true),
    ("SCLASS", "BED", "Bed Shifting;Bed Allotment;Bed Daily Charge", 0, true),
    ("SCLASS", "WARD", "Nursing Charges;Admission", 0, true),
    ("SCLASS", "CASHFLOW", "Advance Deposit;Return Amount;Settle Amount;Advance Refund;Invoice Payment", 0, true),
    ("SCLASS", "PATIENT", "Add Patient;Edit Patient", 0, true),
    ("SCLASS", "PTEST", "Clinical Pathology;Biochemistry;Hematology;Cardiac Path;Serology;Micro Biology;Blood Bank;Hormones & Histopath;Cancer Marker;Cytology;Hormonal & Immunology;Histopath", 0, true),
    ("SCLASS", "RADIO", "CT Scan;MRI;X Ray;Cath Lab;Echo;Sonography USG", 0, true),
    ("SCLASS", "TREATMENT", "Panchkarma;Hydrotherapy;Massage Therapy;Physical Therapy;Yoga;Operation;Procedure", 0, true),
    // RoomType
    ("RoomType", "General", "General", 1, false),
    ("RoomType", "Semi Deluxe", "Semi Deluxe", 2, false),
    ("RoomType", "Deluxe", "Deluxe", 3, false),
    ("RoomType", "Private", "Private", 4, false),
    ("RoomType", "ICU", "ICU", 5, false),
];

// ── charge_category catalog (puru_has.charge_category) ───────────────────────
// (name, type ordinal, status ordinal, applicable, applicable_on_web)
// ChargeCategoryType: REGULAR=0, INTERNAL=2. ChargeCategoryStatus: WAITING_FOR_RATES=1.

const CHARGE_CATEGORIES: &[(&str, u8, u8, bool, bool)] = &[
    ("Normal", 0, 1, true, false),
    ("Online", 2, 1, false, true),
];

// ── puru_document_master catalog (puru_has.puru_document_master) ─────────────
// Main documents: (name, page_size, orientation, height, width, template, format)
// PageSize: A4=4, A5=5, CUSTOM=6. PageOrientation: LANDSCAPE=0, PORTRAIT=1.
// DocumentFormat: PDF=0, XLSX=3, PNG=4. Mirrors setupDocumentTypes + V5/V7 migrations.

const DOC_MASTER_MAIN: &[(&str, u8, u8, i32, i32, &str, u8)] = &[
    ("PuruQRCode", 6, 1, 216, 216, "PuruQRCode.jrxml", 4),
    ("Invoice", 4, 1, 842, 595, "Invoice_Main.jrxml", 0),
    ("Prescription", 4, 1, 842, 595, "OPDMain2.jrxml", 0),
    ("IPD_Admission", 4, 1, 842, 595, "IPD_Admission_1.jrxml", 0),
    ("IPD_Discharge_Summary", 4, 1, 842, 595, "Discharge_Summary_1.jrxml", 0),
    ("FinancialTransaction_Receipt", 5, 0, 420, 595, "Payment_Receipt_2.jrxml", 0),
    ("DailyStats", 4, 1, 842, 595, "DailyStats-2.jrxml", 0),
    ("ExternallyUploadedDoc", 4, 1, 842, 595, "", 3),
    ("ServiceChargeSheet", 4, 1, 842, 595, "", 3),
    ("UploadResultSheet", 4, 1, 842, 595, "", 3),
    ("ADVANCED_INVOICE_REGULAR", 4, 1, 842, 595, "advanced/Invoice_Regular_Main.jrxml", 0),
    ("ADVANCED_INVOICE_AYUSHMAN", 4, 1, 842, 595, "advanced/Invoice_Ayushman_Main.jrxml", 0),
    ("ADVANCED_INVOICE_TPA", 4, 1, 842, 595, "advanced/Invoice_TPA_Main.jrxml", 0),
    ("ADVANCED_ESTIMATE_REGULAR", 4, 1, 842, 595, "advanced/Invoice_Regular_Main.jrxml", 0),
    ("ADVANCED_ESTIMATE_AYUSHMAN", 4, 1, 842, 595, "advanced/Invoice_Ayushman_Main.jrxml", 0),
    ("ADVANCED_ESTIMATE_TPA", 4, 1, 842, 595, "advanced/Invoice_TPA_Main.jrxml", 0),
];

// Sub-reports: (name, template, sr_template_key, sr_parameter_map_key, sr_datasource_key)
const DOC_MASTER_SUB: &[(&str, &str, &str, &str, Option<&str>)] = &[
    ("ActiveServiceStat-SClassList", "ActiveServiceStat-SClassList.jrxml", "SubServiceResultSubReport", "SubServiceResultSubReportParamMap", Some("SubServiceResultSubReportDataSource")),
    ("ActiveServiceStat-ServiceNameList", "ActiveServiceStat-ServiceNameList.jrxml", "SubReport2", "SubReport2ParamMap", Some("SubReport2DataSource")),
    ("FT_Detail_By_User", "FT_Detail_By_User.jrxml", "FTSubReport", "FTSubReportParamMap", Some("FTSubReportDataSource")),
    ("Bill_Type1", "Bill_Type1.jrxml", "SubServiceResultSubReport", "SubServiceResultSubReportParamMap", Some("SubServiceResultSubReportDataSource")),
    ("Bill_Type2", "Bill_Type2.jrxml", "SubServiceResultSubReport", "SubServiceResultSubReportParamMap", Some("SubServiceResultSubReportDataSource")),
    ("PatientInfoHeader", "PatientInfoHeader.jrxml", "PatientHeaderSubReport", "PatientInfoMap", Some("SubDataSource")),
    ("PPINBarcode", "PPINBarcode.jrxml", "PPINBarcodeSubReportTemplate", "PPINBarcodeSubReportParameterMap", Some("PPINBarcodeSubReportDataSource")),
    ("PatientInfoHeaderType2", "PatientInfoHeaderType2.jrxml", "PatientHeaderSubReportTemplate", "PatientHeaderSubReportParameterMap", Some("PatientHeaderSubReportDataSource")),
    ("ConsultantHeader", "ConsultantHeader.jrxml", "ConsultantHeaderSubReportTemplate", "ConsultantHeaderSubReportParameterMap", Some("ConsultantHeaderSubReportDataSource")),
    ("DrugP2", "DrugP2.jrxml", "DrugPrescriptionSubReportTemplate", "DrugPrescriptionSubReportParameterMap", Some("DrugPrescriptionSubReportDataSource")),
    ("Hospital-Name-Header-Hindi", "Hospital-Name-Header-Hindi.jrxml", "SubReport1Template", "SubReport1ParameterMap", Some("SubReport1DataSource")),
    ("Footer1", "Footer1.jrxml", "Footer1Template", "Footer1ParameterMap", Some("Footer1DataSource")),
    ("ADVANCED_SUBREPORT_HEADER_REGULAR", "advanced/SubReport_Header_Regular.jrxml", "HeaderSubReport", "HeaderParams", None),
    ("ADVANCED_SUBREPORT_HEADER_AYUSHMAN", "advanced/SubReport_Header_Ayushman.jrxml", "HeaderSubReport", "HeaderParams", None),
    ("ADVANCED_SUBREPORT_HEADER_TPA", "advanced/SubReport_Header_TPA.jrxml", "HeaderSubReport", "HeaderParams", None),
    ("ADVANCED_SUBREPORT_BODY_DETAIL", "advanced/SubReport_Body_Detail.jrxml", "BodySubReport", "BodyParams", Some("ServiceData")),
    ("ADVANCED_SUBREPORT_BODY_PER_DAY", "advanced/SubReport_Body_PerDay.jrxml", "BodySubReport", "BodyParams", Some("ServiceData")),
    ("ADVANCED_SUBREPORT_BODY_GROUPED", "advanced/SubReport_Body_Grouped.jrxml", "BodySubReport", "BodyParams", Some("ServiceData")),
    ("ADVANCED_SUBREPORT_BODY_SUMMARY", "advanced/SubReport_Body_Summary.jrxml", "BodySubReport", "BodyParams", Some("CategorySummaryData")),
    ("ADVANCED_SUBREPORT_BODY_PACKAGE_ONLY", "advanced/SubReport_Body_PackageOnly.jrxml", "BodySubReport", "BodyParams", Some("PackageData")),
    ("ADVANCED_SUBREPORT_PACKAGE_TABLE", "advanced/SubReport_PackageTable.jrxml", "PackageTableSubReport", "PackageTableParams", Some("PackageData")),
    ("ADVANCED_SUBREPORT_FOOTER", "advanced/SubReport_Footer.jrxml", "FooterSubReport", "FooterParams", None),
];

// ── service rows (puru_has.service) ──────────────────────────────────────────
// Bootstrap services the front-desk workflows rely on (PreloadService maps
// CASHFLOW services by sClass at runtime). Mirrors enterCashflowService and
// submitPatientServices in HASOneTimeController.
// (name, s_class, ServiceType ordinal, internal) — CASHFLOW=6, PATIENT=8.

const HAS_SERVICES: &[(&str, &str, u8, bool)] = &[
    ("Advance Deposit", "Advance Deposit", 6, false),
    ("Settle Amount", "Settle Amount", 6, true),
    ("Return Amount", "Return Amount", 6, false),
    ("Advance Refund", "Advance Refund", 6, false),
    ("Payment Against Invoice", "Invoice Payment", 6, false),
    ("Patient Registration", "Add Patient", 8, false),
    ("Edit Patient", "Edit Patient", 8, false),
];

// ── RabbitMQ queue catalog (vhost from rabbitmq.env) ─────────────────────────
// Union of all @RabbitListener queues across services. Spring services do
// passive declares and crash at boot when a queue is missing.

const QUEUES: &[&str] = &[
    // puru-auth
    "puru_message_to_auth",
    // puru-has listeners (RabbitConfigHAS)
    "send_status_update_to_has",
    "recalculate_ipd_financials",
    "recalculate_visit_financial_info",
    "assign_drug_to_consultant",
    "prepare_document",
    "save_text_suggestion",
    "handle_radio_order",
    "handle_patho_order",
    "send_patho_order",
    "send_radio_order",
    "old_to_new_patient",
    "handle_patient_communication",
    "from_whatsapp_webhook",
    "handle_whatsapp_api_response",
    "upload_has_document_to_cloud",
    "ack_to_has",
    "to_rt_object_transfer",
    "from_rt_object_transfer",
    "intra_puru_master_data_transfer_to_has",
    "intra_puru_master_data_transfer_to_pathology",
    "intra_puru_master_data_transfer_to_radiology",
    "intra_puru_master_data_transfer_to_medical",
    "intra_puru_master_data_transfer_to_counter",
    "handle_consultant_split",
    // puru-pacs / puru-comm
    "ack_report_register",
    "download_report_for_study",
    "has_to_pacs",
    "insert_study_to_fb",
    "insert_study_to_fb_ack",
    "mail_statistics_file",
    "notify_pacs_report_downloaded",
    "pacs_to_has",
    "process_http_entity_xml",
    "q_ct_to_be_archived",
    "q_ct_to_be_archived_dl",
    "q_ct_to_be_uploaded_cloud",
    "q_ct_to_be_uploaded_cloud_dl",
    "q_dicom_to_be_deleted_from_cloud",
    "q_dicom_to_be_deleted_from_cloud_dl",
    "q_integration_inbound_order",
    "q_integration_order_ack",
    "q_integration_patient_sync",
    "q_integration_report_completed",
    "q_integration_scheduling",
    "q_mail_bug",
    "q_mail_pacs_bug",
    "q_pacs_backup",
    "send_notification_01",
    "send_whatsapp_01",
    "upload_dicom_image",
    "upload_dicom_image_retry",
    "upload_dicom_zip_to_cloud",
    "upload_document_to_cloud",
    "upload_document_to_cloud_ack",
];

// ── Entry point ──────────────────────────────────────────────────────────────

pub async fn run_seed(
    seed_db: bool,
    seed_queues: bool,
    seed_templates: bool,
) -> Result<SeedReport, NucleusError> {
    let config = config::load_config()?;
    let mut report = SeedReport::default();

    if seed_db {
        let pool = create_mysql_pool(&config);
        report.sections.push(seed_puru_config(&pool, &config).await);
        report.sections.push(seed_ref_data(&pool).await);
        report.sections.push(seed_charge_categories(&pool).await);
        report.sections.push(seed_document_master(&pool).await);
        report.sections.push(seed_has_services(&pool).await);
        let _ = pool.disconnect().await;
    }

    if seed_queues {
        report.sections.push(seed_rabbitmq_queues(&config).await);
    }

    if seed_templates {
        report.sections.push(seed_jrxml_templates(&config).await);
    }

    Ok(report)
}

fn create_mysql_pool(config: &NucleusConfig) -> mysql_async::Pool {
    let opts = mysql_async::OptsBuilder::default()
        .ip_or_hostname(config.mysql_host.clone())
        .tcp_port(config.mysql_port)
        .user(Some(config.mysql_user.clone()))
        .pass(Some(config.mysql_password.clone()));
    mysql_async::Pool::new(opts)
}

fn generate_secret(salt: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(salt.as_bytes());
    hasher.update(std::process::id().to_le_bytes());
    hasher.update(format!("{:?}", std::time::SystemTime::now()).as_bytes());
    hasher
        .finalize()
        .iter()
        .map(|b| format!("{:02x}", b))
        .collect()
}

// ── DB seeders ───────────────────────────────────────────────────────────────

async fn seed_puru_config(pool: &mysql_async::Pool, config: &NucleusConfig) -> SeedSection {
    let mut section = SeedSection {
        name: "puru_config (puru_auth)".into(),
        created: 0,
        skipped: 0,
        errors: Vec::new(),
    };

    let mut conn = match pool.get_conn().await {
        Ok(c) => c,
        Err(e) => {
            section.errors.push(format!("connect: {}", e));
            return section;
        }
    };

    for (key, value) in config_defaults(config) {
        let result = conn
            .exec_drop(
                "UPDATE puru_auth.puru_config \
                 SET config_value = ? \
                 WHERE config_key = ? AND (config_value IS NULL OR config_value = '')",
                (value.as_str(), key),
            )
            .await;
        match result {
            Ok(()) => {
                if conn.affected_rows() > 0 {
                    section.created += 1;
                } else {
                    section.skipped += 1;
                }
            }
            Err(e) => section.errors.push(format!("{}: {}", key, e)),
        }
    }

    section
}

async fn seed_ref_data(pool: &mysql_async::Pool) -> SeedSection {
    let mut section = SeedSection {
        name: "ref_data (puru_has)".into(),
        created: 0,
        skipped: 0,
        errors: Vec::new(),
    };

    let mut conn = match pool.get_conn().await {
        Ok(c) => c,
        Err(e) => {
            section.errors.push(format!("connect: {}", e));
            return section;
        }
    };

    for (rd_type, key, value, int_value, editable) in REF_DATA {
        // Row identity: (type, string_key) — the RefData entity enforces this
        // as a unique constraint (uc_refdata_type). Never match on the value:
        // values are edited in the UI and re-matching them would attempt a
        // duplicate insert on every subsequent seed run.
        let result = conn
            .exec_drop(
                "INSERT INTO puru_has.ref_data \
                 (type, string_key, string_value, int_value, user_editable) \
                 SELECT ?, ?, ?, ?, ? FROM DUAL \
                 WHERE NOT EXISTS (\
                     SELECT 1 FROM puru_has.ref_data \
                     WHERE type = ? AND string_key = ?\
                 )",
                (*rd_type, *key, *value, *int_value, *editable, *rd_type, *key),
            )
            .await;
        match result {
            Ok(()) => {
                if conn.affected_rows() > 0 {
                    section.created += 1;
                } else {
                    section.skipped += 1;
                }
            }
            Err(e) => section.errors.push(format!("{}/{}: {}", rd_type, key, e)),
        }
    }

    section
}

async fn seed_charge_categories(pool: &mysql_async::Pool) -> SeedSection {
    let mut section = SeedSection {
        name: "charge_category (puru_has)".into(),
        created: 0,
        skipped: 0,
        errors: Vec::new(),
    };

    let mut conn = match pool.get_conn().await {
        Ok(c) => c,
        Err(e) => {
            section.errors.push(format!("connect: {}", e));
            return section;
        }
    };

    for (name, cc_type, status, applicable, on_web) in CHARGE_CATEGORIES {
        let result = conn
            .exec_drop(
                "INSERT IGNORE INTO puru_has.charge_category \
                 (name, type, status, applicable, applicable_on_web) \
                 VALUES (?, ?, ?, ?, ?)",
                (*name, *cc_type, *status, *applicable, *on_web),
            )
            .await;
        match result {
            Ok(()) => {
                if conn.affected_rows() > 0 {
                    section.created += 1;
                } else {
                    section.skipped += 1;
                }
            }
            Err(e) => section.errors.push(format!("{}: {}", name, e)),
        }
    }

    section
}

async fn seed_document_master(pool: &mysql_async::Pool) -> SeedSection {
    let mut section = SeedSection {
        name: "puru_document_master (puru_has)".into(),
        created: 0,
        skipped: 0,
        errors: Vec::new(),
    };

    let mut conn = match pool.get_conn().await {
        Ok(c) => c,
        Err(e) => {
            section.errors.push(format!("connect: {}", e));
            return section;
        }
    };

    for (name, page_size, orientation, height, width, template, format) in DOC_MASTER_MAIN {
        let result = conn
            .exec_drop(
                "INSERT IGNORE INTO puru_has.puru_document_master \
                 (last_modified_by, last_time_updated, height, width, is_sub_report, \
                  name, orientation, page_size, preferred_format, template_name) \
                 VALUES (1, NOW(), ?, ?, 0, ?, ?, ?, ?, ?)",
                (*height, *width, *name, *orientation, *page_size, *format, *template),
            )
            .await;
        match result {
            Ok(()) => {
                if conn.affected_rows() > 0 {
                    section.created += 1;
                } else {
                    section.skipped += 1;
                }
            }
            Err(e) => section.errors.push(format!("{}: {}", name, e)),
        }
    }

    for (name, template, sr_template, sr_params, sr_datasource) in DOC_MASTER_SUB {
        let result = conn
            .exec_drop(
                "INSERT IGNORE INTO puru_has.puru_document_master \
                 (last_modified_by, last_time_updated, height, width, is_sub_report, \
                  name, template_name, sr_template_key, sr_parameter_map_key, sr_datasource_key) \
                 VALUES (1, NOW(), 0, 0, 1, ?, ?, ?, ?, ?)",
                (*name, *template, *sr_template, *sr_params, *sr_datasource),
            )
            .await;
        match result {
            Ok(()) => {
                if conn.affected_rows() > 0 {
                    section.created += 1;
                } else {
                    section.skipped += 1;
                }
            }
            Err(e) => section.errors.push(format!("{}: {}", name, e)),
        }
    }

    section
}

async fn seed_has_services(pool: &mysql_async::Pool) -> SeedSection {
    let mut section = SeedSection {
        name: "service rows (puru_has)".into(),
        created: 0,
        skipped: 0,
        errors: Vec::new(),
    };

    let mut conn = match pool.get_conn().await {
        Ok(c) => c,
        Err(e) => {
            section.errors.push(format!("connect: {}", e));
            return section;
        }
    };

    for (name, s_class, svc_type, internal) in HAS_SERVICES {
        // ID scheme mirrors HASService::getIdForNewService — max id of the
        // type + 1, falling back to ordinal * 1000 + 1 for the first row.
        // operation_status 1 = OperationStatus::ACTIVE.
        let result = conn
            .exec_drop(
                "INSERT INTO puru_has.service \
                 (id, name, s_class, type, internal, allowed_from_web, \
                  multiple_qty_allowed, operation_status, sub_service_present, \
                  for_primary_key, periodic, require_specialist, variable_name, \
                  variable_rate) \
                 SELECT x.next_id, ?, ?, ?, ?, 0, 0, 1, 0, 0, 0, 0, 0, 0 \
                 FROM (\
                     SELECT GREATEST(\
                         COALESCE(MAX(id) + 1, 0), ? * 1000 + 1\
                     ) AS next_id \
                     FROM puru_has.service WHERE type = ?\
                 ) AS x \
                 WHERE NOT EXISTS (SELECT 1 FROM puru_has.service WHERE name = ?)",
                (
                    *name, *s_class, *svc_type, *internal, *svc_type, *svc_type, *name,
                ),
            )
            .await;
        match result {
            Ok(()) => {
                if conn.affected_rows() > 0 {
                    section.created += 1;
                } else {
                    section.skipped += 1;
                }
            }
            Err(e) => section.errors.push(format!("{}: {}", name, e)),
        }
    }

    section
}

// ── RabbitMQ queue seeder ────────────────────────────────────────────────────

fn load_rabbit_settings(config: &NucleusConfig) -> (String, String, String, String) {
    let mut vars: HashMap<String, String> = HashMap::new();
    let path = config.env_dir().join("rabbitmq.env");
    if let Ok(content) = std::fs::read_to_string(&path) {
        for line in content.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if let Some((key, value)) = line.split_once('=') {
                vars.insert(key.trim().to_string(), value.trim().to_string());
            }
        }
    }

    let host = vars
        .get("SPRING_RABBITMQ_HOST")
        .cloned()
        .unwrap_or_else(|| "127.0.0.1".into());
    let vhost = vars
        .get("SPRING_RABBITMQ_VIRTUAL_HOST")
        .cloned()
        .unwrap_or_else(|| "puru".into());
    let user = vars
        .get("SPRING_RABBITMQ_USERNAME")
        .cloned()
        .unwrap_or_else(|| "puru".into());
    let pass = vars
        .get("SPRING_RABBITMQ_PASSWORD")
        .cloned()
        .unwrap_or_else(|| "puru123".into());
    (host, vhost, user, pass)
}

async fn seed_rabbitmq_queues(config: &NucleusConfig) -> SeedSection {
    let mut section = SeedSection {
        name: "rabbitmq queues".into(),
        created: 0,
        skipped: 0,
        errors: Vec::new(),
    };

    let (host, vhost, user, pass) = load_rabbit_settings(config);
    let client = match reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            section.errors.push(format!("http client: {}", e));
            return section;
        }
    };

    // Management plugin API (default port 15672). An existing queue may have
    // been declared with different arguments (x-queue-type, delays, ...) and a
    // blind PUT would 400 on it — so check existence first and never touch
    // queues that already exist.
    let encoded_vhost = vhost.replace('/', "%2F");
    for queue in QUEUES {
        let url = format!(
            "http://{}:15672/api/queues/{}/{}",
            host, encoded_vhost, queue
        );

        let existing = client
            .get(&url)
            .basic_auth(&user, Some(&pass))
            .send()
            .await;
        match existing {
            Ok(resp) if resp.status().is_success() => {
                section.skipped += 1;
                continue;
            }
            Ok(resp) if resp.status().as_u16() == 404 => {} // create below
            Ok(resp) => {
                section
                    .errors
                    .push(format!("{}: HTTP {}", queue, resp.status()));
                continue;
            }
            Err(e) => {
                section.errors.push(format!(
                    "{}: {} (is the RabbitMQ management plugin enabled on {}:15672?)",
                    queue, e, host
                ));
                // Connection-level failure — no point hammering the rest
                break;
            }
        }

        let result = client
            .put(&url)
            .basic_auth(&user, Some(&pass))
            .json(&serde_json::json!({ "durable": true }))
            .send()
            .await;
        match result {
            Ok(resp) if resp.status().is_success() => section.created += 1,
            Ok(resp) => section
                .errors
                .push(format!("{}: HTTP {}", queue, resp.status())),
            Err(e) => {
                section.errors.push(format!("{}: {}", queue, e));
                break;
            }
        }
    }

    section
}

// ── Jasper template seeder ───────────────────────────────────────────────────

async fn seed_jrxml_templates(config: &NucleusConfig) -> SeedSection {
    let mut section = SeedSection {
        name: "jrxml templates".into(),
        created: 0,
        skipped: 0,
        errors: Vec::new(),
    };

    let data_root = config.puru_data_path.as_deref().unwrap_or("");
    if data_root.is_empty() {
        section
            .errors
            .push("puru_data_path is not set in nucleus.toml".into());
        return section;
    }

    match download_templates(data_root).await {
        Ok((created, skipped)) => {
            section.created = created;
            section.skipped = skipped;
        }
        Err(e) => section.errors.push(format!(
            "{} (templates are published as individual objects under \
             gs://<release-bucket>/{}<...> mirroring the config-data/templates \
             directory layout)",
            e.user_message(),
            TEMPLATES_GCS_PREFIX
        )),
    }

    section
}

/// Mirror every template object under the GCS prefix into puru_data, preserving
/// the folder structure. Files already present locally are NEVER overwritten —
/// hospitals customize their jrxml templates and a re-run must not clobber them.
async fn download_templates(data_root: &str) -> Result<(u64, u64), NucleusError> {
    let cred_path = crate::releases::get_credentials_path()?;
    let client = crate::releases::create_gcs_client(&cred_path).await?;

    let objects = crate::releases::list_gcs_objects(&client, TEMPLATES_GCS_PREFIX).await?;

    let root = std::path::Path::new(data_root);
    let (mut created, mut skipped) = (0u64, 0u64);

    for key in objects {
        // The object key is the path relative to puru_data (it already starts
        // with `config-data/templates/`). Build the local path component by
        // component so the '/' separators resolve correctly on every platform.
        let mut local = root.to_path_buf();
        for comp in key.split('/').filter(|c| !c.is_empty()) {
            local.push(comp);
        }

        if local.exists() {
            skipped += 1;
            continue;
        }

        crate::releases::download_gcs_to_file(&client, &key, &local).await?;
        created += 1;
    }

    tracing::info!(
        "Templates: {} new files downloaded under {}, {} existing kept",
        created,
        root.join("config-data").join("templates").display(),
        skipped
    );
    Ok((created, skipped))
}
