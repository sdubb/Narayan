/**
 * BenefitsScroller - Customization Examples
 * 
 * This file demonstrates how to customize the BenefitsScroller component
 * for different industries, use cases, or to add your own stories.
 */

// EXAMPLE 1: Adding a new use case to the array
// ============================================

const newUseCase = {
  id: 9,
  title: 'HR Onboarding Automation',
  description: 'Automate employee onboarding workflows: send welcome emails, create accounts in all systems, set up desk assignments, and notify teams',
  metrics: {
    timeSaved: '22 hours/week',
    moneySaved: '$9,100/month',
    efficiency: '93%'
  },
  icon: '👥',
  color: 'from-rose-500 to-pink-500',
  tools: ['HRIS Integration', 'Email Automation', 'Slack Notifications'],
  story: 'HR team reduced onboarding time from 3 weeks to 2 days. New hires productive on day 1.'
};

// EXAMPLE 2: Industry-specific use cases
// ======================================

const ecommerceUseCases = [
  {
    id: 1,
    title: 'Order Processing & Fulfillment',
    description: 'Automatically process orders from multiple channels, validate inventory, update ShipStation, and notify customers',
    metrics: {
      timeSaved: '35 hours/week',
      moneySaved: '$14,500/month',
      efficiency: '96%'
    },
    icon: '📦',
    color: 'from-indigo-500 to-purple-500',
    tools: ['Shopify', 'ShipStation', 'Email'],
    story: 'Processed 5,000+ orders/month with 99.2% accuracy with zero manual intervention'
  },
  {
    id: 2,
    title: 'Customer Review Monitoring',
    description: 'Monitor reviews across Shopify, Amazon, Google, collect feedback, automate responses, flag for escalation',
    metrics: {
      timeSaved: '12 hours/week',
      moneySaved: '$5,200/month',
      efficiency: '88%'
    },
    icon: '⭐',
    color: 'from-yellow-500 to-orange-500',
    tools: ['Review APIs', 'Sentiment Analysis', 'Slack'],
    story: 'Response time improved from 3 days to <2 hours. CSAT increased 34%'
  }
];

const healthcareUseCases = [
  {
    id: 1,
    title: 'Patient Appointment Scheduling',
    description: 'Schedule appointments, send reminders, handle cancellations, and manage provider calendars automatically',
    metrics: {
      timeSaved: '28 hours/week',
      moneySaved: '$11,800/month',
      efficiency: '94%'
    },
    icon: '🏥',
    color: 'from-red-500 to-rose-500',
    tools: ['EHR System', 'SMS/Email', 'Calendar'],
    story: 'No-show rate reduced from 18% to 6%. Scheduling capacity increased 40%'
  },
  {
    id: 2,
    title: 'Insurance Authorization Processing',
    description: 'Process prior authorizations: verify coverage, submit requests, track status, notify providers and patients',
    metrics: {
      timeSaved: '45 hours/week',
      moneySaved: '$18,900/month',
      efficiency: '97%'
    },
    icon: '📋',
    color: 'from-blue-500 to-cyan-500',
    tools: ['Insurance APIs', 'EHR', 'Communication'],
    story: 'Authorization turnaround reduced from 5 days to 2 hours. Approved 98% first submission'
  }
];

const legalUseCases = [
  {
    id: 1,
    title: 'Contract Review Pipeline',
    description: 'Upload contracts, extract key terms, identify risk clauses, generate executive summaries, route for approval',
    metrics: {
      timeSaved: '38 hours/week',
      moneySaved: '$24,500/month',
      efficiency: '95%'
    },
    icon: '⚖️',
    color: 'from-slate-600 to-slate-800',
    tools: ['Document AI', 'Workflow Engine', 'Approval System'],
    story: '200 contracts reviewed monthly. Critical risks caught in 99.2% of cases. Reduced review cycle by 60%'
  }
];

const financialServicesUseCases = [
  {
    id: 1,
    title: 'KYC/AML Compliance Review',
    description: 'Trigger KYC workflows on new accounts, verify identity documents, check sanctions lists, generate reports',
    metrics: {
      timeSaved: '52 hours/week',
      moneySaved: '$21,700/month',
      efficiency: '98%'
    },
    icon: '🔐',
    color: 'from-amber-600 to-orange-600',
    tools: ['Identity Verification', 'Sanctions Databases', 'Compliance Reporting'],
    story: 'Onboarded 10,000+ accounts monthly. AML false positive rate reduced 65%. Zero compliance violations'
  },
  {
    id: 2,
    title: 'Trade Settlement & Reconciliation',
    description: 'Automate settlement workflows, reconcile trades, flag mismatches, generate settlement reports',
    metrics: {
      timeSaved: '40 hours/week',
      moneySaved: '$16,800/month',
      efficiency: '99%'
    },
    icon: '📊',
    color: 'from-green-600 to-emerald-600',
    tools: ['Trading Systems', 'Clearing Houses', 'Settlement Agents'],
    story: 'Settled $2.3B in trades/month with 99.8% accuracy. Zero settlement delays'
  }
];

// EXAMPLE 3: Formatting metrics
// =============================

// Good metric formats:
const goodMetrics = {
  timeSaved: '20 hours/week',     // Clear format
  moneySaved: '$8,500/month',     // Currency + timeframe
  efficiency: '95%'               // Percentage
};

// Bad (will break calculation):
const badMetrics = {
  timeSaved: '20 hours',          // Missing timeframe
  moneySaved: '8.5K per month',   // Non-standard currency format
  efficiency: 'Very High'         // Not a percentage
};

// EXAMPLE 4: Color palette matching
// ==================================

const colorPalettes = {
  // Primary brand colors
  primary: 'from-blue-500 to-cyan-500',
  secondary: 'from-purple-500 to-pink-500',
  
  // Industry colors
  finance: 'from-amber-600 to-orange-600',
  healthcare: 'from-red-500 to-rose-500',
  legal: 'from-slate-600 to-slate-800',
  ecommerce: 'from-indigo-500 to-purple-500',
  technology: 'from-green-500 to-emerald-500',
  marketing: 'from-rose-500 to-pink-500',
  operations: 'from-yellow-500 to-amber-500',
  
  // Emotional colors
  success: 'from-green-500 to-emerald-500',
  warning: 'from-yellow-500 to-orange-500',
  critical: 'from-red-500 to-rose-500',
  neutral: 'from-slate-400 to-slate-600'
};

// EXAMPLE 5: Creating a modified component for specific industries
// =================================================================

// For Healthcare Brand
const HealthcareSpecificScroller = () => {
  const [useCases] = useState(healthcareUseCases);
  // ... rest of component with healthcare styling
};

// For Finance Brand
const FinanceSpecificScroller = () => {
  const [useCases] = useState(financialServicesUseCases);
  // ... rest of component with finance styling
};

// EXAMPLE 6: Dynamic metric calculation
// ======================================

function calculateSummaryMetrics(useCases) {
  const totalHours = useCases.reduce((sum, c) => {
    const hours = parseInt(c.metrics.timeSaved);
    return sum + hours;
  }, 0);
  
  const totalMoney = useCases.reduce((sum, c) => {
    const money = parseInt(c.metrics.moneySaved);
    return sum + money;
  }, 0);
  
  const totalEfficiency = useCases.reduce((sum, c) => {
    const eff = parseInt(c.metrics.efficiency);
    return sum + eff;
  }, 0);
  
  return {
    avgHours: (totalHours / useCases.length).toFixed(0),
    avgMoney: ((totalMoney / useCases.length) / 1000).toFixed(0),
    avgEfficiency: (totalEfficiency / useCases.length).toFixed(0)
  };
}

// Usage:
const metrics = calculateSummaryMetrics(ecommerceUseCases);
// Result: { avgHours: '23', avgMoney: '9', avgEfficiency: '92' }

// EXAMPLE 7: Custom styling per use case
// =======================================

const advancedUseCase = {
  id: 1,
  title: 'Advanced ML-Driven Process',
  description: 'Uses machine learning to predict outcomes and optimize workflows in real-time',
  metrics: {
    timeSaved: '50 hours/week',
    moneySaved: '$25,000/month',
    efficiency: '98%'
  },
  icon: '🤖',
  color: 'from-cyan-500 via-blue-500 to-purple-500',  // Three-color gradient
  tools: ['ML Pipeline', 'Real-time Data', 'Predictive Analytics'],
  story: 'Achieved 98% prediction accuracy. Prevented $2M in losses through proactive intervention',
  
  // Optional: Custom styling override
  customStyles: {
    cardMinHeight: '520px',
    animationType: 'pulse',  // or 'fade', 'slide'
    highlightColor: '#00d9ff'
  }
};

// EXAMPLE 8: Story creation tips
// ===============================

// Good stories (specific, measurable, impactful):
const goodStories = [
  'Reduced payment processing time from 3 days to 4 hours. Recovered $500K in overdue invoices',
  'Support team cleared 2-week backlog in 3 days. CSAT improved 22%',
  'Sales team focused on 200 qualified leads instead of 5,000. Deal cycle reduced 35%',
  'Processed 50,000 invoices/month with 99.7% accuracy. Zero customer disputes'
];

// Weak stories (vague, unmeasurable):
const weakStories = [
  'Much faster processing',
  'Improved customer satisfaction',
  'Better team productivity'
];

// EXAMPLE 9: Seasonal use cases
// ==============================

const seasonalUseCases = {
  holiday: {
    id: 1,
    title: 'Holiday Season Order Surge Management',
    description: 'Handle 10x order volume during holidays with automated fulfillment and customer service',
    metrics: {
      timeSaved: '80 hours/week',  // During peak
      moneySaved: '$35,000/month',
      efficiency: '94%'
    },
    icon: '🎄',
    color: 'from-red-600 to-green-600',
    tools: ['Order Management', 'Inventory', 'Customer Service'],
    story: 'Handled 50,000 orders during Black Friday. Zero fulfillment delays. 98% positive reviews'
  },
  
  tax: {
    id: 2,
    title: 'Tax Season Document Processing',
    description: 'Automate tax document collection, verification, and filing workflows',
    metrics: {
      timeSaved: '120 hours/week',  // March-April peak
      moneySaved: '$42,000/month',
      efficiency: '97%'
    },
    icon: '📊',
    color: 'from-amber-500 to-orange-600',
    tools: ['Document Collection', 'Verification', 'E-filing'],
    story: 'Processed 10,000 tax returns in 6 weeks. 100% filing compliance. Zero IRS errors'
  }
};

// EXAMPLE 10: Performance metrics tracking
// =========================================

// Track which use cases are most engaging
const analytics = {
  trackCardViews: (caseId) => {
    console.log(`User viewed use case: ${caseId}`);
  },
  
  trackCTAClick: (caseId) => {
    console.log(`User clicked CTA from use case: ${caseId}`);
  },
  
  trackScroll: (direction) => {
    console.log(`User scrolled ${direction}`);
  },
  
  getMostEngagingCase: (viewCounts) => {
    return Object.keys(viewCounts).reduce((a, b) => 
      viewCounts[a.id] > viewCounts[b.id] ? a : b
    );
  }
};

export {
  newUseCase,
  ecommerceUseCases,
  healthcareUseCases,
  legalUseCases,
  financialServicesUseCases,
  seasonalUseCases,
  colorPalettes,
  calculateSummaryMetrics,
  goodStories,
  analytics
};
