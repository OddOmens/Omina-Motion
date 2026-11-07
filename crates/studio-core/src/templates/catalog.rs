//! The 61 templates: one for each layout, each with its own words, colors and font, and
//! the palettes they can be recolored with.
//! Words are short on purpose, so they fit the narrow formats; line breaks are written in.

use super::{Category, Layout, Palette, Template};

/// The palettes templates use.
const MIDNIGHT: Palette = Palette { background: 0x0b1020, background_2: 0x1b2350, accent: 0x7c5cff, text: 0xffffff, muted: 0xa5acc9, on_accent: 0xffffff };
const SUNSET: Palette = Palette { background: 0x2a0f1e, background_2: 0x5b1a2e, accent: 0xff7a45, text: 0xffffff, muted: 0xf2c4b5, on_accent: 0x1a0a10 };
const LIME: Palette = Palette { background: 0x0d0d0d, background_2: 0x1f1f1f, accent: 0xd4f25c, text: 0xffffff, muted: 0xa3a3a3, on_accent: 0x111111 };
const CREAM: Palette = Palette { background: 0xf6efe6, background_2: 0xe9dccb, accent: 0xc2410c, text: 0x1c1917, muted: 0x6b5e53, on_accent: 0xffffff };
const OCEAN: Palette = Palette { background: 0x03263b, background_2: 0x0b5a7a, accent: 0x2dd4bf, text: 0xffffff, muted: 0x9fd3e0, on_accent: 0x042f2e };
const BLUSH: Palette = Palette { background: 0xfdf2f8, background_2: 0xfbcfe8, accent: 0xdb2777, text: 0x3b0a24, muted: 0x9d5a7b, on_accent: 0xffffff };
const FOREST: Palette = Palette { background: 0x0f1f17, background_2: 0x1f3d2c, accent: 0x86efac, text: 0xf0fdf4, muted: 0xa7c4b3, on_accent: 0x052e16 };
const MONO: Palette = Palette { background: 0xffffff, background_2: 0xe5e5e5, accent: 0x111111, text: 0x111111, muted: 0x6b6b6b, on_accent: 0xffffff };
const ELECTRIC: Palette = Palette { background: 0x1e1b4b, background_2: 0x4c1d95, accent: 0xfacc15, text: 0xffffff, muted: 0xc4b5fd, on_accent: 0x1e1b4b };
const CORAL: Palette = Palette { background: 0xfff1ec, background_2: 0xffd8c9, accent: 0xff5a36, text: 0x2b1a14, muted: 0x8a6152, on_accent: 0xffffff };
const GOLD: Palette = Palette { background: 0x111111, background_2: 0x2b2416, accent: 0xe8b44a, text: 0xfff8e7, muted: 0xbfa77a, on_accent: 0x111111 };
const SKY: Palette = Palette { background: 0xeff6ff, background_2: 0xbfdbfe, accent: 0x2563eb, text: 0x0f172a, muted: 0x475569, on_accent: 0xffffff };
const RED: Palette = Palette { background: 0x1a0505, background_2: 0x450a0a, accent: 0xef4444, text: 0xffffff, muted: 0xfca5a5, on_accent: 0xffffff };
const NEON_NIGHT: Palette = Palette { background: 0x07021a, background_2: 0x1a0b3d, accent: 0xff2bd6, text: 0xffffff, muted: 0xb8a6ff, on_accent: 0x1a0026 };
const CANDY: Palette = Palette { background: 0xfff0f6, background_2: 0xffd6e7, accent: 0xff4d8d, text: 0x3a0d22, muted: 0x9b5874, on_accent: 0xffffff };
const MINT: Palette = Palette { background: 0xecfdf5, background_2: 0xa7f3d0, accent: 0x059669, text: 0x064e3b, muted: 0x3f7d66, on_accent: 0xffffff };
const SUNNY: Palette = Palette { background: 0xfffbeb, background_2: 0xfde68a, accent: 0xf59e0b, text: 0x451a03, muted: 0x92400e, on_accent: 0x1c1917 };
const PARTY: Palette = Palette { background: 0x2e1065, background_2: 0x7c3aed, accent: 0xfacc15, text: 0xffffff, muted: 0xddd6fe, on_accent: 0x2e1065 };
const CHAT_LIGHT: Palette = Palette { background: 0xffffff, background_2: 0xe9e9eb, accent: 0x0a84ff, text: 0x111111, muted: 0x6b6b6b, on_accent: 0xffffff };
const CHAT_DARK: Palette = Palette { background: 0x000000, background_2: 0x26252a, accent: 0x0a84ff, text: 0xffffff, muted: 0x8e8e93, on_accent: 0xffffff };

/// Flat, bold palettes for the editorial layouts: one strong color and one ink.
const COBALT: Palette = Palette { background: 0x1739e6, background_2: 0x6d8bff, accent: 0xffffff, text: 0xffffff, muted: 0xc9d4ff, on_accent: 0x1739e6 };
const TOMATO: Palette = Palette { background: 0xff4a1c, background_2: 0xff8a5c, accent: 0x111111, text: 0x111111, muted: 0x5c1a08, on_accent: 0xff4a1c };
const CITRUS: Palette = Palette { background: 0xffe03a, background_2: 0xffa800, accent: 0x111111, text: 0x111111, muted: 0x5a4a00, on_accent: 0xffe03a };
const INK: Palette = Palette { background: 0x0b0b0b, background_2: 0x262626, accent: 0xff5a1f, text: 0xffffff, muted: 0x9a9a9a, on_accent: 0x0b0b0b };
const PAPER: Palette = Palette { background: 0xf3efe6, background_2: 0xe2dccf, accent: 0x2347ff, text: 0x121212, muted: 0x6d6a63, on_accent: 0xffffff };
const LILAC: Palette = Palette { background: 0xe7b9ff, background_2: 0xc58bff, accent: 0x111111, text: 0x111111, muted: 0x5b3a70, on_accent: 0xe7b9ff };
const DEEP: Palette = Palette { background: 0x05060f, background_2: 0x10162e, accent: 0x3b82f6, text: 0xffffff, muted: 0x94a3b8, on_accent: 0xffffff };

/// Every palette by name, for choosing a template's colors in the Templates panel.
pub const LOOKS: [(&str, Palette); 27] = [
    ("Midnight", MIDNIGHT),
    ("Sunset", SUNSET),
    ("Lime", LIME),
    ("Cream", CREAM),
    ("Ocean", OCEAN),
    ("Blush", BLUSH),
    ("Forest", FOREST),
    ("Mono", MONO),
    ("Electric", ELECTRIC),
    ("Coral", CORAL),
    ("Gold", GOLD),
    ("Sky", SKY),
    ("Red", RED),
    ("Neon Night", NEON_NIGHT),
    ("Candy", CANDY),
    ("Mint", MINT),
    ("Sunny", SUNNY),
    ("Party", PARTY),
    ("Chat Light", CHAT_LIGHT),
    ("Chat Dark", CHAT_DARK),
    ("Cobalt", COBALT),
    ("Tomato", TOMATO),
    ("Citrus", CITRUS),
    ("Ink", INK),
    ("Paper", PAPER),
    ("Lilac", LILAC),
    ("Deep", DEEP),
];

/// Every template, one for each layout. Its colors and font are a starting point: the
/// Templates panel can give any template another palette (`LOOKS`) or a brand's look.
pub static TEMPLATES: [Template; 61] = [
    Template { name: "Launch Day", category: Category::Marketing, layout: Layout::Headline, copy: &["INTRODUCING", "Meet the\nNew Standard", "Built for teams that move fast", "Learn More"], palette: MIDNIGHT, font: ("Avenir Next", 800), seconds: 12.0 },
    Template { name: "Flash Deal", category: Category::Sales, layout: Layout::SplitPromo, copy: &["Weekend\nDeal", "Today only", "$29", "Shop Now"], palette: CORAL, font: ("Futura", 700), seconds: 10.0 },
    Template { name: "3 Reasons", category: Category::Education, layout: Layout::List, copy: &["3 Reasons to\nSwitch", "Saves 5 hours a week", "Works with your tools", "Free for 30 days", "Try It Free"], palette: SKY, font: ("Helvetica Neue", 800), seconds: 12.0 },
    Template { name: "Founder Quote", category: Category::Podcast, layout: Layout::Quote, copy: &["We build what\nwe wish existed.", "Alex Rivera", "Founder, Northwind"], palette: CREAM, font: ("Georgia", 700), seconds: 10.0 },
    Template { name: "Product Drop", category: Category::Product, layout: Layout::Countdown, copy: &["DROPPING IN", "NOW LIVE", "Link in bio"], palette: RED, font: ("Impact", 400), seconds: 10.0 },
    Template { name: "Interview Name", category: Category::Ugc, layout: Layout::LowerThird, copy: &["Maya Chen", "Product Designer"], palette: OCEAN, font: ("Avenir Next", 700), seconds: 10.0 },
    Template { name: "Five Stars", category: Category::Testimonials, layout: Layout::Review, copy: &["Changed how I\nwork every day.", "Priya S.", "Studio Pro"], palette: SUNSET, font: ("Avenir Next", 700), seconds: 10.0 },
    Template { name: "Half Off", category: Category::Sales, layout: Layout::SaleBurst, copy: &["50%", "OFF", "Everything must go", "CODE: HALF50"], palette: RED, font: ("Impact", 400), seconds: 10.0 },
    Template { name: "POV Hook", category: Category::Ugc, layout: Layout::Hook, copy: &["POV: you finally\nfound the one", "Wait for it…"], palette: MONO, font: ("Helvetica Neue", 800), seconds: 10.0 },
    Template { name: "New Sneaker", category: Category::Product, layout: Layout::ProductCard, copy: &["Air Runner", "$129", "Lightweight · Breathable", "Shop Now"], palette: CORAL, font: ("Futura", 700), seconds: 12.0 },
    Template { name: "Save the Date", category: Category::Events, layout: Layout::Event, copy: &["SAVE THE DATE", "Summer Gala", "JUNE 21", "Grand Hall, Austin"], palette: GOLD, font: ("Didot", 700), seconds: 12.0 },
    Template { name: "Growth Stat", category: Category::Marketing, layout: Layout::BigStat, copy: &["3×", "more engagement", "Based on 2,000 posts"], palette: LIME, font: ("Avenir Next", 800), seconds: 10.0 },
    Template { name: "Glow Up", category: Category::Ugc, layout: Layout::BeforeAfter, copy: &["BEFORE", "AFTER", "The glow up is real"], palette: BLUSH, font: ("Avenir Next", 800), seconds: 10.0 },
    Template { name: "How It Works", category: Category::Education, layout: Layout::Steps, copy: &["How It Works", "Pick a plan", "Set it up", "Enjoy"], palette: SKY, font: ("Helvetica Neue", 800), seconds: 14.0 },
    Template { name: "New Feature", category: Category::Product, layout: Layout::Announcement, copy: &["NEW", "Dark Mode\nIs Here", "Update the app today"], palette: FOREST, font: ("Avenir Next", 800), seconds: 10.0 },
    Template { name: "Episode Promo", category: Category::Podcast, layout: Layout::Podcast, copy: &["THE GROWTH SHOW", "Building in\nPublic", "with Taylor Brooks"], palette: GOLD, font: ("Avenir Next", 800), seconds: 15.0 },
    Template { name: "Logo Reveal", category: Category::Brand, layout: Layout::BrandIntro, copy: &["NORTHWIND", "Made to move"], palette: MIDNIGHT, font: ("Avenir Next", 800), seconds: 10.0 },
    Template { name: "Weekend Recap", category: Category::Ugc, layout: Layout::Collage, copy: &["Weekend\nRecap", "Best moments"], palette: CREAM, font: ("Futura", 700), seconds: 12.0 },
    Template { name: "Sale Marquee", category: Category::Sales, layout: Layout::Marquee, copy: &["SALE", "Up to 60% off"], palette: RED, font: ("Impact", 400), seconds: 10.0 },
    Template { name: "Follow for More", category: Category::Social, layout: Layout::EndCard, copy: &["Follow for\nMore", "@yourhandle", "Follow"], palette: CORAL, font: ("Avenir Next", 800), seconds: 10.0 },
    Template { name: "Group Chat", category: Category::Fun, layout: Layout::Chat, copy: &["Bestie", "did you see the\nnew drop??", "omg YES", "we need it", "already ordered"], palette: CHAT_DARK, font: ("Helvetica Neue", 800), seconds: 12.0 },
    Template { name: "Order Shipped", category: Category::Product, layout: Layout::Notification, copy: &["SHOP", "Your order shipped!", "Arrives Thursday", "Track It Live"], palette: SKY, font: ("Helvetica Neue", 800), seconds: 10.0 },
    Template { name: "Viral Post", category: Category::Social, layout: Layout::PostCard, copy: &["Alex Rivera", "@alexmakes", "nobody:\nme at 2 AM: one more edit", "24.8K"], palette: MONO, font: ("Helvetica Neue", 800), seconds: 10.0 },
    Template { name: "Coffee or Tea", category: Category::Fun, layout: Layout::ThisOrThat, copy: &["Which one?", "Coffee", "Tea"], palette: CANDY, font: ("Futura", 700), seconds: 10.0 },
    Template { name: "Monday Meme", category: Category::Fun, layout: Layout::Meme, copy: &["ME ON MONDAY", "ME ON FRIDAY"], palette: MONO, font: ("Impact", 400), seconds: 10.0 },
    Template { name: "Coming Soon Type", category: Category::Marketing, layout: Layout::Typewriter, copy: &["Something new is coming...", "Stay tuned"], palette: MONO, font: ("American Typewriter", 700), seconds: 10.0 },
    Template { name: "Open Late", category: Category::Brand, layout: Layout::Neon, copy: &["OPEN", "Late night menu till 2 AM"], palette: NEON_NIGHT, font: ("Futura", 700), seconds: 10.0 },
    Template { name: "Happy Birthday", category: Category::Celebrations, layout: Layout::Confetti, copy: &["Happy\nBirthday!", "Make a wish"], palette: PARTY, font: ("Arial Rounded MT Bold", 700), seconds: 10.0 },
    Template { name: "App Launch Phone", category: Category::Product, layout: Layout::PhoneMockup, copy: &["Meet Focus", "The timer that keeps\nyou in the zone", "Download"], palette: OCEAN, font: ("Avenir Next", 800), seconds: 12.0 },
    Template { name: "Summer Memories", category: Category::Social, layout: Layout::Polaroid, copy: &["summer '26", "The best days"], palette: SUNNY, font: ("Futura", 700), seconds: 10.0 },
    Template { name: "Trivia Time", category: Category::Education, layout: Layout::Quiz, copy: &["Fastest land animal?", "Lion", "Cheetah", "Horse"], palette: ELECTRIC, font: ("Futura", 700), seconds: 10.0 },
    Template { name: "Now Playing", category: Category::Podcast, layout: Layout::NowPlaying, copy: &["Midnight Drive", "The Night Shift"], palette: NEON_NIGHT, font: ("Helvetica Neue", 800), seconds: 12.0 },
    Template { name: "Fundraiser Goal", category: Category::Events, layout: Layout::Progress, copy: &["Fundraiser Goal", "75%", "$7,500 of $10,000 raised"], palette: MINT, font: ("Avenir Next", 800), seconds: 10.0 },
    Template { name: "Big Bold Loud", category: Category::Marketing, layout: Layout::KineticStack, copy: &["BIG", "BOLD", "LOUD", "NOW"], palette: MONO, font: ("Impact", 400), seconds: 10.0 },
    Template { name: "We've Moved", category: Category::Brand, layout: Layout::MapPin, copy: &["We've Moved!", "45 Oak Street, Austin"], palette: CREAM, font: ("Georgia", 700), seconds: 10.0 },
    Template { name: "Welcome Coupon", category: Category::Sales, layout: Layout::Coupon, copy: &["Welcome Gift", "15%", "WELCOME15", "For your first order"], palette: SKY, font: ("Helvetica Neue", 800), seconds: 10.0 },
    Template { name: "System On", category: Category::Brand, layout: Layout::GlitchTitle, copy: &["SYSTEM ON", "Welcome back"], palette: NEON_NIGHT, font: ("Impact", 400), seconds: 10.0 },
    // Intros and social formats (see `social`).
    Template { name: "Meet Me", category: Category::Intros, layout: Layout::ProfileIntro, copy: &["Hi, I'm", "Maya", "@mayamakes", "Designer", "Coffee fan", "Dog mom"], palette: BLUSH, font: ("Avenir Next", 800), seconds: 12.0 },
    Template { name: "Ask Me Anything", category: Category::Social, layout: Layout::AskMeAnything, copy: &["ASK ME ANYTHING", "What camera do\nyou film with?", "Just my phone, honestly"], palette: ELECTRIC, font: ("Avenir Next", 800), seconds: 10.0 },
    Template { name: "Name Drop", category: Category::Intros, layout: Layout::NameReveal, copy: &["Allow me to introduce", "JORDAN", "Creator · Editor · Dreamer"], palette: LIME, font: ("Helvetica Neue", 800), seconds: 10.0 },
    Template { name: "Story Post", category: Category::Social, layout: Layout::StoryFrame, copy: &["yourname", "2h", "Sunday reset", "Send message"], palette: CANDY, font: ("Helvetica Neue", 700), seconds: 10.0 },
    Template { name: "Comment Reply", category: Category::Social, layout: Layout::CommentReply, copy: &["@jess", "How do you edit\nso fast?", "Okay, here's my secret"], palette: CHAT_LIGHT, font: ("Helvetica Neue", 800), seconds: 10.0 },
    Template { name: "Birthday Wishes", category: Category::Celebrations, layout: Layout::FloatingHearts, copy: &["Happy Birthday", "Sophia", "Wishing you your best year yet"], palette: CANDY, font: ("Snell Roundhand", 700), seconds: 12.0 },
    Template { name: "Photo Dump", category: Category::Social, layout: Layout::OutlineStack, copy: &["15", "PHOTOS", "TRENDING"], palette: SUNNY, font: ("Impact", 400), seconds: 10.0 },
    Template { name: "Vlog Intro", category: Category::Intros, layout: Layout::VlogTitle, copy: &["VLOG", "a day in my life", "DAY 12"], palette: SKY, font: ("Arial Rounded MT Bold", 700), seconds: 10.0 },
    Template { name: "Honest Review", category: Category::Ugc, layout: Layout::UgcCaptions, copy: &["I tried the viral\nserum for 30 days", "Week one? Nothing.", "Week three, my skin\nwas actually glowing", "Would I buy it again?\nAlready did.", "Shop the Serum"], palette: MONO, font: ("Helvetica Neue", 800), seconds: 13.0 },
    Template { name: "Us vs Them", category: Category::Ads, layout: Layout::VersusTable, copy: &["Why People\nSwitch", "Us", "Others", "Free shipping", "30-day returns", "Real support", "Shop Now"], palette: MIDNIGHT, font: ("Avenir Next", 800), seconds: 11.0 },
    Template { name: "Problem Solved", category: Category::Ads, layout: Layout::ProblemSolution, copy: &["THE PROBLEM", "Cables\neverywhere", "THE FIX", "One dock.\nZero mess.", "Shop the Dock"], palette: LIME, font: ("Helvetica Neue", 800), seconds: 11.0 },
    Template { name: "Feature Tour", category: Category::Ads, layout: Layout::FeatureCallouts, copy: &["Meet Pocket Cam", "4K at 60 fps", "All-day battery", "Fits any pocket", "Pre-order"], palette: SUNNY, font: ("Avenir Next", 800), seconds: 11.0 },
    Template { name: "Flash Sale Clock", category: Category::Sales, layout: Layout::OfferTimer, copy: &["FLASH SALE", "40% OFF", "Everything, sitewide", "Shop the Sale"], palette: RED, font: ("Impact", 400), seconds: 12.0 },
    Template { name: "Rated 4.9", category: Category::Testimonials, layout: Layout::SocialProof, copy: &["Loved by\nBusy Parents", "4.9", "from 12,000+ reviews", "See Why"], palette: CREAM, font: ("Georgia", 700), seconds: 11.0 },
    Template { name: "Poster Stretch", category: Category::Events, layout: Layout::PosterStretch, copy: &["All Eyes\non\nIndie Film", "January 19–21", "indiefest.com", "Tickets →"], palette: INK, font: ("Avenir Next Condensed", 800), seconds: 10.0 },
    Template { name: "Sliding Blocks", category: Category::Events, layout: Layout::SlidingBlocks, copy: &["SPECIAL EVENT", "Wonderlust", "Sept 9 · 10 AM PT"], palette: PAPER, font: ("Helvetica Neue", 800), seconds: 10.0 },
    Template { name: "Type Repeater", category: Category::Brand, layout: Layout::TypeRepeater, copy: &["MOVE", "New collection · Out now"], palette: TOMATO, font: ("Avenir Next Condensed", 800), seconds: 10.0 },
    Template { name: "Card Stack", category: Category::Testimonials, layout: Layout::CardStack, copy: &["“Easily the best thing\nI bought this year.”", "Maya R.", "Verified buyer"], palette: LILAC, font: ("Helvetica Neue", 700), seconds: 10.0 },
    Template { name: "Bold Countdown", category: Category::Celebrations, layout: Layout::BoldCountdown, copy: &["Happy\nNew Year", "See you in 2027"], palette: CITRUS, font: ("Avenir Next Condensed", 800), seconds: 10.0 },
    Template { name: "Blur Spin", category: Category::Intros, layout: Layout::BlurSpin, copy: &["Available in\nSpatial Audio", "Listen now"], palette: DEEP, font: ("Helvetica Neue", 700), seconds: 10.0 },
    Template { name: "Split Reveal", category: Category::Social, layout: Layout::SplitReveal, copy: &["Behind\nthe Scenes", "EPISODE 04"], palette: INK, font: ("Avenir Next Condensed", 800), seconds: 10.0 },
    Template { name: "Gradient Title", category: Category::Marketing, layout: Layout::GradientTitle, copy: &["Love\nwins.", "Pride Month · Join us"], palette: COBALT, font: ("Helvetica Neue", 800), seconds: 10.0 },
    Template { name: "Photo Tagline", category: Category::Ads, layout: Layout::PhotoTagline, copy: &["FRESHLY", "BAKED", "THE CRUST", "Order Now"], palette: CITRUS, font: ("Impact", 400), seconds: 10.0 },
    Template { name: "Bouncy Period", category: Category::Brand, layout: Layout::BouncyPeriod, copy: &["Done", "Simple tools. No fuss."], palette: PAPER, font: ("Helvetica Neue", 800), seconds: 10.0 },
];
