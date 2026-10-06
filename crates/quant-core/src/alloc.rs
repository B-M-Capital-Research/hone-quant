//! Capped proportional allocation ("water-filling").
//!
//! Distributes a budget proportionally to non-negative scores while respecting per-item caps.
//! Whatever a capped item cannot absorb is redistributed to the uncapped items in proportion to
//! their scores, so the result is the unique allocation `w_i = min(cap_i, λ·s_i)` with
//! `Σw = min(budget, Σ_{s_i>0} cap_i)`.

/// Allocates `budget` across items. Items with a non-positive score or cap receive zero.
pub fn capped_proportional(scores: &[f64], caps: &[f64], budget: f64) -> Vec<f64> {
    assert_eq!(scores.len(), caps.len(), "scores and caps must align");
    let n = scores.len();
    let mut weights = vec![0.0; n];
    if !(budget > 0.0) {
        return weights;
    }
    let mut active: Vec<usize> = (0..n)
        .filter(|&i| scores[i] > 0.0 && scores[i].is_finite() && caps[i] > 0.0)
        .collect();
    let mut remaining = budget;
    // Each pass either finishes or permanently caps at least one item, so n passes suffice.
    for _ in 0..=n {
        if active.is_empty() || remaining <= 1e-15 {
            break;
        }
        let total_score: f64 = active.iter().map(|&i| scores[i]).sum();
        let lambda = remaining / total_score;
        let newly_capped: Vec<usize> = active
            .iter()
            .copied()
            .filter(|&i| lambda * scores[i] >= caps[i])
            .collect();
        if newly_capped.is_empty() {
            for &i in &active {
                weights[i] = lambda * scores[i];
            }
            break;
        }
        for &i in &newly_capped {
            weights[i] = caps[i];
            remaining -= caps[i];
        }
        active.retain(|i| !newly_capped.contains(i));
    }
    weights
}

/// Like [`capped_proportional`] but every eligible item first receives `min(floor, cap)`.
/// Floors are scaled down uniformly if they alone exceed the budget.
pub fn floored_capped_proportional(
    scores: &[f64],
    caps: &[f64],
    floor: f64,
    budget: f64,
) -> Vec<f64> {
    assert_eq!(scores.len(), caps.len(), "scores and caps must align");
    if !(budget > 0.0) {
        return vec![0.0; scores.len()];
    }
    let eligible: Vec<bool> = scores
        .iter()
        .zip(caps)
        .map(|(s, c)| *s > 0.0 && s.is_finite() && *c > 0.0)
        .collect();
    let mut floors: Vec<f64> = caps
        .iter()
        .zip(&eligible)
        .map(|(c, e)| if *e { floor.max(0.0).min(*c) } else { 0.0 })
        .collect();
    let floor_total: f64 = floors.iter().sum();
    if floor_total > budget {
        let scale = budget / floor_total;
        floors.iter_mut().for_each(|f| *f *= scale);
        return floors;
    }
    let residual_caps: Vec<f64> = caps.iter().zip(&floors).map(|(c, f)| c - f).collect();
    let extra = capped_proportional(scores, &residual_caps, budget - floor_total);
    floors.iter().zip(extra).map(|(f, e)| f + e).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sum(xs: &[f64]) -> f64 {
        xs.iter().sum()
    }

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-12
    }

    #[test]
    fn uncapped_allocation_is_proportional() {
        let w = capped_proportional(&[1.0, 3.0], &[1.0, 1.0], 0.8);
        assert!(close(w[0], 0.2));
        assert!(close(w[1], 0.6));
    }

    #[test]
    fn caps_redistribute_to_the_rest() {
        let w = capped_proportional(&[1.0, 1.0, 8.0], &[0.5, 0.5, 0.3], 0.9);
        assert!(close(w[2], 0.3));
        assert!(close(w[0], 0.3));
        assert!(close(w[1], 0.3));
        assert!(close(sum(&w), 0.9));
    }

    #[test]
    fn cascading_caps_converge() {
        // After capping the big item, the medium one also exceeds its cap.
        let w = capped_proportional(&[10.0, 5.0, 1.0, 1.0], &[0.2, 0.2, 1.0, 1.0], 1.0);
        assert!(close(w[0], 0.2));
        assert!(close(w[1], 0.2));
        assert!(close(w[2], 0.3));
        assert!(close(w[3], 0.3));
    }

    #[test]
    fn total_capacity_below_budget_leaves_cash() {
        let w = capped_proportional(&[1.0, 1.0], &[0.1, 0.2], 1.0);
        assert!(close(w[0], 0.1));
        assert!(close(w[1], 0.2));
    }

    #[test]
    fn zero_scores_and_caps_receive_nothing() {
        let w = capped_proportional(&[0.0, 2.0, f64::NAN, 1.0], &[1.0, 1.0, 1.0, 0.0], 0.5);
        assert_eq!(w, vec![0.0, 0.5, 0.0, 0.0]);
        assert_eq!(capped_proportional(&[1.0], &[1.0], 0.0), vec![0.0]);
    }

    #[test]
    fn floors_are_granted_first() {
        let w = floored_capped_proportional(&[1.0, 9.0], &[1.0, 1.0], 0.2, 1.0);
        // Floors 0.2 each, remaining 0.6 split 1:9.
        assert!(close(w[0], 0.26));
        assert!(close(w[1], 0.74));
    }

    #[test]
    fn floors_scale_down_when_budget_is_short() {
        let w = floored_capped_proportional(&[1.0, 1.0, 1.0], &[1.0, 1.0, 1.0], 0.5, 0.9);
        assert!(w.iter().all(|x| close(*x, 0.3)));
    }

    #[test]
    fn floor_respects_item_cap() {
        let w = floored_capped_proportional(&[1.0, 1.0], &[0.05, 1.0], 0.1, 0.5);
        assert!(close(w[0], 0.05));
        assert!(close(w[1], 0.45));
    }
}
