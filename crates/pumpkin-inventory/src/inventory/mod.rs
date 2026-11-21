use pumpkin_data::item_stack::ItemStack;
use std::sync::RwLock;

#[expect(clippy::module_inception)]
mod inventory;
mod simple_inventory;

pub use inventory::*;
pub use simple_inventory::*;

/// [`Inventory::update_stack`] for slots behind one lock: the write lock is held from read to
/// write, so a parallel write can't land in between and get overwritten.
// TODO `Inventory::update_stack` promises a write-back only on `true`, but the callback runs on the
// stored slot, so a callback that mutates and returns `false` still changes it. No caller does.
pub fn update_locked_stack<S: AsMut<[ItemStack]> + ?Sized>(
    stacks: &RwLock<S>,
    slot: usize,
    update: &mut dyn FnMut(&mut ItemStack) -> bool,
) -> bool {
    stacks
        .write()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .as_mut()
        .get_mut(slot)
        .is_some_and(update)
}

// Utility functions for split_stack
pub fn split_stack_slice(stacks: &mut [ItemStack], slot: usize, amount: u8) -> ItemStack {
    if !stacks[slot].is_empty() && amount > 0 {
        stacks[slot].split(amount)
    } else {
        ItemStack::EMPTY.clone()
    }
}

/// Replaces a slot behind one lock, holding the write lock across the write.
pub fn set_locked_stack<S: AsMut<[ItemStack]> + ?Sized>(
    stacks: &RwLock<S>,
    slot: usize,
    stack: ItemStack,
) {
    stacks
        .write()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .as_mut()[slot] = stack;
}

/// Splits `amount` off a slot behind one lock, using the same slot handling as [`split_stack_slice`].
pub fn split_locked_stack<S: AsMut<[ItemStack]> + ?Sized>(
    stacks: &RwLock<S>,
    slot: usize,
    amount: u8,
) -> ItemStack {
    let mut stacks = stacks
        .write()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    split_stack_slice(stacks.as_mut(), slot, amount)
}
