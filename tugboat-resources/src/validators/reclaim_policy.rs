use crate::validators::Validator;

const VALID_RECLAIM_POLICIES: &[&str] = &["Delete", "Retain"];

pub trait HasReclaimPolicy {
    fn reclaim_policy_value(&self) -> Option<&str>;
}

pub struct ReclaimPolicyValidator;

impl<T> Validator<T> for ReclaimPolicyValidator
where
    T: HasReclaimPolicy,
{
    fn validate(&self, value: &T) -> bool {
        match value.reclaim_policy_value() {
            None => true,
            Some(policy) => VALID_RECLAIM_POLICIES.contains(&policy),
        }
    }
}
