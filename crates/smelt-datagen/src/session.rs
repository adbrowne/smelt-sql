//! Session summary table generator.

use crate::gen::Gen;
use crate::generators::*;
use chrono::NaiveDate;
use rand::{Rng, RngCore, SeedableRng};
use rand_chacha::ChaCha8Rng;
use std::sync::Arc;
use uuid::Uuid;

/// Platform types for sessions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Platform {
    WebDesktop,
    Android,
    Ios,
    WebMobile,
}

impl Platform {
    pub fn as_str(&self) -> &'static str {
        match self {
            Platform::WebDesktop => "web_desktop",
            Platform::Android => "android",
            Platform::Ios => "ios",
            Platform::WebMobile => "web_mobile",
        }
    }
}

/// Visit source types.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VisitSource {
    Seo,
    Sem,
    Direct,
    Referral,
    Affiliate,
    Email,
    Social,
    OrganicSocial,
}

impl VisitSource {
    pub fn as_str(&self) -> &'static str {
        match self {
            VisitSource::Seo => "seo",
            VisitSource::Sem => "sem",
            VisitSource::Direct => "direct",
            VisitSource::Referral => "referral",
            VisitSource::Affiliate => "affiliate",
            VisitSource::Email => "email",
            VisitSource::Social => "social",
            VisitSource::OrganicSocial => "organic_social",
        }
    }

    /// Whether this source should have a campaign associated.
    pub fn has_campaign(&self) -> bool {
        matches!(
            self,
            VisitSource::Sem | VisitSource::Referral | VisitSource::Affiliate | VisitSource::Email
        )
    }
}

/// Product categories.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProductCategory {
    Electronics,
    Clothing,
    Home,
    Sports,
    Beauty,
    Food,
}

impl ProductCategory {
    pub fn as_str(&self) -> &'static str {
        match self {
            ProductCategory::Electronics => "electronics",
            ProductCategory::Clothing => "clothing",
            ProductCategory::Home => "home",
            ProductCategory::Sports => "sports",
            ProductCategory::Beauty => "beauty",
            ProductCategory::Food => "food",
        }
    }

    /// Average price per item in this category (in cents).
    pub fn avg_price(&self) -> i32 {
        match self {
            ProductCategory::Electronics => 15000, // $150
            ProductCategory::Clothing => 5000,     // $50
            ProductCategory::Home => 8000,         // $80
            ProductCategory::Sports => 7000,       // $70
            ProductCategory::Beauty => 3000,       // $30
            ProductCategory::Food => 2000,         // $20
        }
    }
}

/// User status categories.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UserStatus {
    New,
    High,
    Medium,
    Low,
}

impl UserStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            UserStatus::New => "new",
            UserStatus::High => "high",
            UserStatus::Medium => "medium",
            UserStatus::Low => "low",
        }
    }
}

/// A visitor with sticky attributes.
#[derive(Debug, Clone)]
pub struct Visitor {
    pub id: Uuid,
    pub platform_preference: Platform,
    pub return_probability: f64,
    pub country: &'static str,
    pub user_status: UserStatus,
}

/// A session record.
#[derive(Debug, Clone)]
pub struct Session {
    pub visitor_id: Uuid,
    pub session_id: Uuid,
    pub platform: Platform,
    pub visit_source: VisitSource,
    pub visit_campaign: Option<String>,
    pub widget_views: i32,
    pub session_date: NaiveDate,
    pub product_views: i32,
    pub product_category: ProductCategory,
    pub product_revenue: i32,
    pub product_purchase_count: i32,
    pub country: String,
    pub user_status: UserStatus,
}

/// Shared visitor pool that can be cloned across parallel workers.
#[derive(Clone)]
pub struct VisitorPool {
    visitors: Arc<Vec<Visitor>>,
}

impl VisitorPool {
    /// Create a visitor pool from a seed.
    pub fn new(seed: u64, target_sessions: usize) -> Self {
        let mut rng = ChaCha8Rng::seed_from_u64(seed);
        // Assume average 3-7 sessions per visitor over the period
        let num_visitors = target_sessions / 5;
        let visitors = generate_visitors(&mut rng, num_visitors);
        Self {
            visitors: Arc::new(visitors),
        }
    }

    /// Get the number of visitors in the pool.
    pub fn len(&self) -> usize {
        self.visitors.len()
    }

    /// Check if the pool is empty.
    pub fn is_empty(&self) -> bool {
        self.visitors.is_empty()
    }
}

/// Generate deterministic per-day seeds from a root seed.
pub fn generate_day_seeds(root_seed: u64, num_days: u32) -> Vec<u64> {
    // Use offset to ensure these seeds don't overlap with visitor pool generation
    let mut rng = ChaCha8Rng::seed_from_u64(root_seed.wrapping_add(1000));
    (0..num_days).map(|_| rng.next_u64()).collect()
}

/// Configuration for generating a single day's sessions.
pub struct DayGenerator {
    visitor_pool: VisitorPool,
    day_seed: u64,
    date: NaiveDate,
    sessions_per_day: usize,
}

impl DayGenerator {
    /// Create a new day generator.
    pub fn new(
        visitor_pool: VisitorPool,
        day_seed: u64,
        date: NaiveDate,
        sessions_per_day: usize,
    ) -> Self {
        Self {
            visitor_pool,
            day_seed,
            date,
            sessions_per_day,
        }
    }

    /// Generate all sessions for this day, returning a Vec.
    pub fn generate(&self) -> Vec<Session> {
        let mut rng = ChaCha8Rng::seed_from_u64(self.day_seed);
        let mut sessions = Vec::new();

        // Sample visitors for this day based on return probability
        let mut daily_visitor_indices: Vec<usize> = Vec::new();

        for (idx, visitor) in self.visitor_pool.visitors.iter().enumerate() {
            // Higher return probability = more likely to visit any given day
            let daily_visit_prob = 0.05 + visitor.return_probability * 0.15;
            if rng.gen_bool(daily_visit_prob.min(1.0)) {
                daily_visitor_indices.push(idx);
            }
        }

        // If we don't have enough visitors, sample more randomly
        while daily_visitor_indices.len() < self.sessions_per_day / 2 {
            let idx = rng.gen_range(0..self.visitor_pool.visitors.len());
            if !daily_visitor_indices.contains(&idx) {
                daily_visitor_indices.push(idx);
            }
        }

        // Generate sessions for each visitor
        for visitor_idx in &daily_visitor_indices {
            let visitor = &self.visitor_pool.visitors[*visitor_idx];
            // 1-3 sessions per visitor per day
            let num_sessions = rng.gen_range(1..=3);

            for _ in 0..num_sessions {
                let session_rows = self.generate_session(&mut rng, visitor);
                sessions.extend(session_rows);

                if sessions.len() >= self.sessions_per_day {
                    return sessions;
                }
            }
        }

        sessions
    }

    fn generate_session(&self, rng: &mut ChaCha8Rng, visitor: &Visitor) -> Vec<Session> {
        let mut sessions = Vec::new();

        let session_id = uuid_gen().generate(rng);

        // Platform: 90% follows preference, 10% random
        let platform = if rng.gen_bool(0.90) {
            visitor.platform_preference
        } else {
            platform_gen().generate(rng)
        };

        let visit_source = visit_source_gen().generate(rng);
        let visit_campaign = if visit_source.has_campaign() {
            Some(campaign_gen().generate(rng))
        } else {
            None
        };

        // Widget views: log-normal, median ~5
        let widget_views = log_normal(5.0, 1.0, 100).generate(rng);

        // Generate 1-4 categories for this session (average ~2)
        let num_categories = {
            let r: f64 = rng.gen();
            if r < 0.30 {
                1
            } else if r < 0.70 {
                2
            } else if r < 0.90 {
                3
            } else {
                4
            }
        };

        // Select distinct categories for this session
        let mut selected_categories: Vec<ProductCategory> = Vec::with_capacity(num_categories);
        while selected_categories.len() < num_categories {
            let cat = product_category_gen().generate(rng);
            if !selected_categories.contains(&cat) {
                selected_categories.push(cat);
            }
        }

        // Generate a row for each category
        for &product_category in &selected_categories {
            // Product views: log-normal, median ~3 (split across categories)
            let product_views = log_normal(3.0 / num_categories as f64, 1.0, 50).generate(rng);

            // Purchase: 80% zero, otherwise geometric
            let product_purchase_count = if rng.gen_bool(0.80) {
                0
            } else {
                geometric(0.5).generate(rng) + 1
            };

            // Revenue based on purchase count and category price
            let product_revenue = if product_purchase_count > 0 {
                let base_price = product_category.avg_price();
                let price_factor = rng.gen_range(0.5..1.5);
                (product_purchase_count as f64 * base_price as f64 * price_factor) as i32
            } else {
                0
            };

            sessions.push(Session {
                visitor_id: visitor.id,
                session_id,
                platform,
                visit_source,
                visit_campaign: visit_campaign.clone(),
                widget_views,
                session_date: self.date,
                product_views,
                product_category,
                product_revenue,
                product_purchase_count,
                country: visitor.country.to_string(),
                user_status: visitor.user_status,
            });
        }

        sessions
    }
}

/// Generate the visitor pool.
fn generate_visitors(rng: &mut impl Rng, count: usize) -> Vec<Visitor> {
    let uuid_g = uuid_gen();
    let platform_g = platform_gen();
    let country_g = country_gen();
    let user_status_g = user_status_gen();

    (0..count)
        .map(|_| {
            let id = uuid_g.generate(rng);
            let platform_preference = platform_g.generate(rng);
            // Power-law distribution for return probability
            let return_probability = rng.gen::<f64>().powf(2.0) * 0.8;
            let country = country_g.generate(rng);
            let user_status = user_status_g.generate(rng);

            Visitor {
                id,
                platform_preference,
                return_probability,
                country,
                user_status,
            }
        })
        .collect()
}

/// Campaign names (30 distinct values).
const CAMPAIGNS: &[&str] = &[
    "summer_sale_2024",
    "winter_promo",
    "black_friday",
    "cyber_monday",
    "spring_clearance",
    "new_arrivals",
    "loyalty_rewards",
    "flash_sale_jan",
    "flash_sale_feb",
    "flash_sale_mar",
    "brand_launch",
    "influencer_collab",
    "email_exclusive",
    "app_install",
    "retargeting_cart",
    "retargeting_browse",
    "holiday_special",
    "back_to_school",
    "mothers_day",
    "fathers_day",
    "valentines",
    "easter_promo",
    "labor_day",
    "memorial_day",
    "new_year",
    "free_shipping",
    "bogo_deal",
    "clearance_final",
    "vip_early_access",
    "referral_bonus",
];

/// Generator for the visitor's platform preference.
fn platform_gen() -> WeightedChoice<Platform> {
    weighted_choice(vec![
        (Platform::WebDesktop, 0.40),
        (Platform::Android, 0.25),
        (Platform::Ios, 0.20),
        (Platform::WebMobile, 0.15),
    ])
}

/// Generator for visit source.
fn visit_source_gen() -> WeightedChoice<VisitSource> {
    weighted_choice(vec![
        (VisitSource::Seo, 0.30),
        (VisitSource::Direct, 0.25),
        (VisitSource::Sem, 0.15),
        (VisitSource::Referral, 0.10),
        (VisitSource::Affiliate, 0.08),
        (VisitSource::Email, 0.07),
        (VisitSource::Social, 0.03),
        (VisitSource::OrganicSocial, 0.02),
    ])
}

/// Generator for product category.
fn product_category_gen() -> WeightedChoice<ProductCategory> {
    weighted_choice(vec![
        (ProductCategory::Electronics, 0.20),
        (ProductCategory::Clothing, 0.25),
        (ProductCategory::Home, 0.15),
        (ProductCategory::Sports, 0.15),
        (ProductCategory::Beauty, 0.15),
        (ProductCategory::Food, 0.10),
    ])
}

/// Generator for country (population-weighted EU member states).
fn country_gen() -> WeightedChoice<&'static str> {
    weighted_choice(vec![
        ("DE", 0.19),  // Germany
        ("FR", 0.15),  // France
        ("IT", 0.14),  // Italy
        ("ES", 0.11),  // Spain
        ("PL", 0.09),  // Poland
        ("RO", 0.04),  // Romania
        ("NL", 0.04),  // Netherlands
        ("BE", 0.03),  // Belgium
        ("GR", 0.02),  // Greece
        ("CZ", 0.02),  // Czechia
        ("PT", 0.02),  // Portugal
        ("SE", 0.02),  // Sweden
        ("HU", 0.02),  // Hungary
        ("AT", 0.02),  // Austria
        ("BG", 0.01),  // Bulgaria
        ("DK", 0.01),  // Denmark
        ("FI", 0.01),  // Finland
        ("SK", 0.01),  // Slovakia
        ("IE", 0.01),  // Ireland
        ("HR", 0.01),  // Croatia
        ("LT", 0.005), // Lithuania
        ("SI", 0.005), // Slovenia
        ("LV", 0.004), // Latvia
        ("EE", 0.003), // Estonia
        ("CY", 0.002), // Cyprus
        ("LU", 0.001), // Luxembourg
        ("MT", 0.001), // Malta
    ])
}

/// Generator for user status.
fn user_status_gen() -> WeightedChoice<UserStatus> {
    weighted_choice(vec![
        (UserStatus::New, 0.20),
        (UserStatus::High, 0.15),
        (UserStatus::Medium, 0.35),
        (UserStatus::Low, 0.30),
    ])
}

/// Generator for campaign names.
fn campaign_gen() -> OneOf<String> {
    one_of(CAMPAIGNS.iter().map(|s| s.to_string()).collect())
}

/// Session generator configuration and state.
pub struct SessionGenerator {
    start_date: NaiveDate,
    num_days: u32,
    target_sessions: usize,
    visitors: Vec<Visitor>,
}

impl SessionGenerator {
    /// Create a new session generator.
    ///
    /// # Arguments
    /// * `seed` - Random seed for deterministic generation
    /// * `start_date` - First day of the date range
    /// * `num_days` - Number of days to generate sessions for
    /// * `target_sessions` - Approximate total number of sessions to generate
    pub fn new(seed: u64, start_date: NaiveDate, num_days: u32, target_sessions: usize) -> Self {
        let mut rng = ChaCha8Rng::seed_from_u64(seed);

        // Calculate number of visitors needed
        // Assume average 3-7 sessions per visitor over the period
        let num_visitors = target_sessions / 5;
        let visitors = generate_visitors(&mut rng, num_visitors);

        Self {
            start_date,
            num_days,
            target_sessions,
            visitors,
        }
    }

    /// Generate all sessions as an iterator.
    pub fn generate(self, seed: u64) -> SessionIterator {
        SessionIterator::new(self, seed)
    }
}

/// Iterator that yields sessions in batches.
/// Each session can have multiple categories, yielding multiple rows with the same session_id.
pub struct SessionIterator {
    config: SessionGenerator,
    rng: ChaCha8Rng,
    current_day: u32,
    sessions_generated: usize,
    daily_visitor_indices: Vec<usize>,
    daily_visitor_pos: usize,
    sessions_per_visitor: usize,
    session_in_visit: usize,
    /// Pending category rows for current session
    pending_categories: Vec<Session>,
}

impl SessionIterator {
    fn new(config: SessionGenerator, seed: u64) -> Self {
        let rng = ChaCha8Rng::seed_from_u64(seed.wrapping_add(1)); // Different seed from visitor generation

        Self {
            config,
            rng,
            current_day: 0,
            sessions_generated: 0,
            daily_visitor_indices: Vec::new(),
            daily_visitor_pos: 0,
            sessions_per_visitor: 0,
            session_in_visit: 0,
            pending_categories: Vec::new(),
        }
    }

    fn setup_day(&mut self) {
        // Calculate how many sessions we need per day on average
        let sessions_per_day = self.config.target_sessions / self.config.num_days as usize;

        // Sample visitors for this day based on return probability
        self.daily_visitor_indices.clear();

        for (idx, visitor) in self.config.visitors.iter().enumerate() {
            // Higher return probability = more likely to visit any given day
            // Base probability is low, scaled by return_probability
            let daily_visit_prob = 0.05 + visitor.return_probability * 0.15;
            if self.rng.gen_bool(daily_visit_prob.min(1.0)) {
                self.daily_visitor_indices.push(idx);
            }
        }

        // If we don't have enough visitors, sample more randomly
        while self.daily_visitor_indices.len() < sessions_per_day / 2 {
            let idx = self.rng.gen_range(0..self.config.visitors.len());
            if !self.daily_visitor_indices.contains(&idx) {
                self.daily_visitor_indices.push(idx);
            }
        }

        self.daily_visitor_pos = 0;
    }
}

impl Iterator for SessionIterator {
    type Item = Session;

    fn next(&mut self) -> Option<Session> {
        // First, return any pending category rows
        if let Some(session) = self.pending_categories.pop() {
            return Some(session);
        }

        if self.sessions_generated >= self.config.target_sessions {
            return None;
        }

        // Check if we need to set up a new day
        if self.current_day >= self.config.num_days {
            return None;
        }

        if self.daily_visitor_indices.is_empty()
            || self.daily_visitor_pos >= self.daily_visitor_indices.len()
        {
            if self.current_day > 0 && self.daily_visitor_pos >= self.daily_visitor_indices.len() {
                self.current_day += 1;
                if self.current_day >= self.config.num_days {
                    return None;
                }
            }
            self.setup_day();
            if self.current_day == 0 {
                self.current_day = 1;
            }
        }

        // Generate session for current visitor
        if self.session_in_visit >= self.sessions_per_visitor || self.sessions_per_visitor == 0 {
            // Move to next visitor or generate new session count
            if self.session_in_visit > 0 {
                self.daily_visitor_pos += 1;
                if self.daily_visitor_pos >= self.daily_visitor_indices.len() {
                    self.current_day += 1;
                    if self.current_day >= self.config.num_days {
                        return None;
                    }
                    self.setup_day();
                }
            }
            // 1-3 sessions per visitor per day
            self.sessions_per_visitor = self.rng.gen_range(1..=3);
            self.session_in_visit = 0;
        }

        let visitor_idx = self.daily_visitor_indices[self.daily_visitor_pos];
        let visitor = &self.config.visitors[visitor_idx];

        // Generate session
        let session_id = uuid_gen().generate(&mut self.rng);

        // Platform: 90% follows preference, 10% random
        let platform = if self.rng.gen_bool(0.90) {
            visitor.platform_preference
        } else {
            platform_gen().generate(&mut self.rng)
        };

        let visit_source = visit_source_gen().generate(&mut self.rng);
        let visit_campaign = if visit_source.has_campaign() {
            Some(campaign_gen().generate(&mut self.rng))
        } else {
            None
        };

        // Widget views: log-normal, median ~5
        let widget_views = log_normal(5.0, 1.0, 100).generate(&mut self.rng);

        let session_date =
            self.config.start_date + chrono::Duration::days((self.current_day - 1) as i64);

        // Generate 1-4 categories for this session (average ~2)
        // Distribution: 30% get 1, 40% get 2, 20% get 3, 10% get 4
        let num_categories = {
            let r: f64 = self.rng.gen();
            if r < 0.30 {
                1
            } else if r < 0.70 {
                2
            } else if r < 0.90 {
                3
            } else {
                4
            }
        };

        // Select distinct categories for this session
        // There are 6 categories total, so num_categories (1-4) is always achievable
        let mut selected_categories: Vec<ProductCategory> = Vec::with_capacity(num_categories);
        while selected_categories.len() < num_categories {
            let cat = product_category_gen().generate(&mut self.rng);
            if !selected_categories.contains(&cat) {
                selected_categories.push(cat);
            }
        }

        // Generate a row for each category
        let mut first_session: Option<Session> = None;
        for (i, &product_category) in selected_categories.iter().enumerate() {
            // Product views: log-normal, median ~3 (split across categories)
            let product_views =
                log_normal(3.0 / num_categories as f64, 1.0, 50).generate(&mut self.rng);

            // Purchase: 80% zero, otherwise geometric
            let product_purchase_count = if self.rng.gen_bool(0.80) {
                0
            } else {
                geometric(0.5).generate(&mut self.rng) + 1
            };

            // Revenue based on purchase count and category price
            let product_revenue = if product_purchase_count > 0 {
                let base_price = product_category.avg_price();
                // Add some variance: 0.5x to 1.5x base price
                let price_factor = self.rng.gen_range(0.5..1.5);
                (product_purchase_count as f64 * base_price as f64 * price_factor) as i32
            } else {
                0
            };

            let session = Session {
                visitor_id: visitor.id,
                session_id,
                platform,
                visit_source,
                visit_campaign: visit_campaign.clone(),
                widget_views,
                session_date,
                product_views,
                product_category,
                product_revenue,
                product_purchase_count,
                country: visitor.country.to_string(),
                user_status: visitor.user_status,
            };

            if i == 0 {
                // First category will be returned directly
                first_session = Some(session);
            } else {
                // Additional categories go to pending
                self.pending_categories.push(session);
            }
        }

        self.session_in_visit += 1;
        self.sessions_generated += 1;

        first_session
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_deterministic_generation() {
        let start = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();
        let gen1 = SessionGenerator::new(42, start, 30, 1000);
        let sessions1: Vec<_> = gen1.generate(42).take(100).collect();

        let gen2 = SessionGenerator::new(42, start, 30, 1000);
        let sessions2: Vec<_> = gen2.generate(42).take(100).collect();

        assert_eq!(sessions1.len(), sessions2.len());
        for (s1, s2) in sessions1.iter().zip(sessions2.iter()) {
            assert_eq!(s1.visitor_id, s2.visitor_id);
            assert_eq!(s1.session_id, s2.session_id);
            assert_eq!(s1.platform, s2.platform);
        }
    }

    #[test]
    fn test_campaign_only_for_relevant_sources() {
        let start = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();
        let gen = SessionGenerator::new(42, start, 30, 10000);

        for session in gen.generate(42).take(1000) {
            if session.visit_source.has_campaign() {
                assert!(session.visit_campaign.is_some());
            } else {
                assert!(session.visit_campaign.is_none());
            }
        }
    }

    #[test]
    fn test_revenue_correlates_with_purchases() {
        let start = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();
        let gen = SessionGenerator::new(42, start, 30, 10000);

        for session in gen.generate(42).take(1000) {
            if session.product_purchase_count == 0 {
                assert_eq!(session.product_revenue, 0);
            } else {
                assert!(session.product_revenue > 0);
            }
        }
    }
}
