use compio_buf::BufResult;

use crate::{DriverType, Extra, Key, OpCode, key::ErasedKey};

/// The end of a heterogeneous list.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct HNil;

/// A single member of a heterogeneous list.
///
/// Each member's type and the list's length are known at compile time. Use
/// [`hlist!`](crate::hlist) to construct a balanced list and
/// [`hlist_pat!`](crate::hlist_pat) to destructure it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct HLeaf<T> {
    /// The member.
    pub value: T,
}

/// Two ordered subtrees of a heterogeneous list.
///
/// Members in `left` precede members in `right`. The list macros combine
/// adjacent pairs at each level, keeping the tree depth logarithmic.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct HBranch<Left, Right> {
    /// The earlier members.
    pub left: Left,
    /// The later members.
    pub right: Right,
}

/// How an operation is linked to the next member of its chain.
///
/// The last member is always unlinked, regardless of its `Link` value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Link {
    /// Cancel the remaining chain if this operation fails or completes short.
    Soft,
    /// Continue the chain even if this operation fails or completes short.
    Hard,
}

/// Construct a heterogeneous list in the given order.
///
/// ```
/// use compio_driver::{hlist, hlist_pat};
///
/// let hlist_pat![number, text] = hlist![42, "hello"];
/// assert_eq!(number, 42);
/// assert_eq!(text, "hello");
/// ```
#[macro_export]
macro_rules! hlist {
    () => { $crate::HNil };
    ($($value:expr),+ $(,)?) => {
        $crate::__hlist_balance!(@reduce [] [$(($crate::HLeaf { value: $value }))*])
    };
}

/// Destructure a heterogeneous list in the given order.
///
/// The number and types of patterns must match the list at compile time.
/// See [`hlist!`](crate::hlist) for an example.
#[macro_export]
macro_rules! hlist_pat {
    () => { $crate::HNil };
    ($($value:pat),+ $(,)?) => {
        $crate::__hlist_balance!(@reduce [] [$(($crate::HLeaf { value: $value }))*])
    };
}

// Values and patterns use the same token-tree reduction, so their shapes agree.
// Consume sixteen pairs per expansion to keep even 1,024 members below the
// default macro recursion limit. Smaller power-of-two batches handle
// remainders.
#[doc(hidden)]
#[macro_export]
macro_rules! __hlist_balance {
    (@reduce [$root:tt] []) => { $root };
    (@reduce [$($done:tt)*] []) => {
        $crate::__hlist_balance!(@reduce [] [$($done)*])
    };
    (@reduce [$($done:tt)*] [
        $a:tt $b:tt $c:tt $d:tt $e:tt $f:tt $g:tt $h:tt
        $i:tt $j:tt $k:tt $l:tt $m:tt $n:tt $o:tt $p:tt
        $q:tt $r:tt $s:tt $t:tt $u:tt $v:tt $w:tt $x:tt
        $y:tt $z:tt $aa:tt $bb:tt $cc:tt $dd:tt $ee:tt $ff:tt
        $($rest:tt)*
    ]) => {
        $crate::__hlist_balance!(@reduce [
            $($done)*
            ($crate::HBranch { left: $a, right: $b })
            ($crate::HBranch { left: $c, right: $d })
            ($crate::HBranch { left: $e, right: $f })
            ($crate::HBranch { left: $g, right: $h })
            ($crate::HBranch { left: $i, right: $j })
            ($crate::HBranch { left: $k, right: $l })
            ($crate::HBranch { left: $m, right: $n })
            ($crate::HBranch { left: $o, right: $p })
            ($crate::HBranch { left: $q, right: $r })
            ($crate::HBranch { left: $s, right: $t })
            ($crate::HBranch { left: $u, right: $v })
            ($crate::HBranch { left: $w, right: $x })
            ($crate::HBranch { left: $y, right: $z })
            ($crate::HBranch { left: $aa, right: $bb })
            ($crate::HBranch { left: $cc, right: $dd })
            ($crate::HBranch { left: $ee, right: $ff })
        ] [$($rest)*])
    };
    (@reduce [$($done:tt)*] [
        $a:tt $b:tt $c:tt $d:tt $e:tt $f:tt $g:tt $h:tt
        $i:tt $j:tt $k:tt $l:tt $m:tt $n:tt $o:tt $p:tt
        $($rest:tt)*
    ]) => {
        $crate::__hlist_balance!(@reduce [
            $($done)*
            ($crate::HBranch { left: $a, right: $b })
            ($crate::HBranch { left: $c, right: $d })
            ($crate::HBranch { left: $e, right: $f })
            ($crate::HBranch { left: $g, right: $h })
            ($crate::HBranch { left: $i, right: $j })
            ($crate::HBranch { left: $k, right: $l })
            ($crate::HBranch { left: $m, right: $n })
            ($crate::HBranch { left: $o, right: $p })
        ] [$($rest)*])
    };
    (@reduce [$($done:tt)*] [
        $a:tt $b:tt $c:tt $d:tt $e:tt $f:tt $g:tt $h:tt $($rest:tt)*
    ]) => {
        $crate::__hlist_balance!(@reduce [
            $($done)*
            ($crate::HBranch { left: $a, right: $b })
            ($crate::HBranch { left: $c, right: $d })
            ($crate::HBranch { left: $e, right: $f })
            ($crate::HBranch { left: $g, right: $h })
        ] [$($rest)*])
    };
    (@reduce [$($done:tt)*] [$a:tt $b:tt $c:tt $d:tt $($rest:tt)*]) => {
        $crate::__hlist_balance!(@reduce [
            $($done)*
            ($crate::HBranch { left: $a, right: $b })
            ($crate::HBranch { left: $c, right: $d })
        ] [$($rest)*])
    };
    (@reduce [$($done:tt)*] [$left:tt $right:tt $($rest:tt)*]) => {
        $crate::__hlist_balance!(@reduce [
            $($done)* ($crate::HBranch { left: $left, right: $right })
        ] [$($rest)*])
    };
    (@reduce [$($done:tt)*] [$last:tt]) => {
        $crate::__hlist_balance!(@reduce [$($done)* $last] [])
    };
}

mod sealed {
    pub trait Ops {}
    pub trait Keys {}
}

/// A statically typed list of `(operation, link_to_next)` members.
///
/// Implemented for [`HNil`], [`HLeaf`] members whose operation implements
/// [`OpCode`], and [`HBranch`] subtrees. This trait is sealed: callers
/// construct lists with [`hlist!`](crate::hlist), rather than implement it
/// themselves.
pub trait LinkedOps: sealed::Ops + Sized {
    /// The corresponding list of typed keys and their links.
    type Keys: LinkedKeys<Ops = Self>;

    /// The number of operations.
    const LEN: usize;

    #[doc(hidden)]
    fn into_keys(
        self,
        extras: &mut impl FnMut() -> Extra,
        entries: &mut impl FnMut(ErasedKey, Link),
    ) -> Self::Keys;
}

/// The statically typed keys returned by [`crate::Proactor::push_linked`].
///
/// Each member contains `(Key<Op>, Link)`, preserving the operation types
/// and submission order. This trait is sealed.
pub trait LinkedKeys: sealed::Keys + Sized {
    /// The corresponding list of operations and links.
    type Ops: LinkedOps<Keys = Self>;

    #[doc(hidden)]
    fn into_ops(self) -> Self::Ops;
}

impl sealed::Ops for HNil {}
impl sealed::Keys for HNil {}

impl LinkedOps for HNil {
    type Keys = HNil;

    const LEN: usize = 0;

    fn into_keys(
        self,
        _: &mut impl FnMut() -> Extra,
        _: &mut impl FnMut(ErasedKey, Link),
    ) -> Self::Keys {
        HNil
    }
}

impl LinkedKeys for HNil {
    type Ops = HNil;

    fn into_ops(self) -> Self::Ops {
        HNil
    }
}

impl<Op: OpCode + 'static> sealed::Ops for HLeaf<(Op, Link)> {}

impl<Op: OpCode + 'static> LinkedOps for HLeaf<(Op, Link)> {
    type Keys = HLeaf<(Key<Op>, Link)>;

    const LEN: usize = 1;

    fn into_keys(
        self,
        extras: &mut impl FnMut() -> Extra,
        entries: &mut impl FnMut(ErasedKey, Link),
    ) -> Self::Keys {
        let (op, link) = self.value;
        let key = Key::new(op, extras(), DriverType::IoUring);
        entries(key.clone().erase(), link);
        HLeaf { value: (key, link) }
    }
}

impl<Left: LinkedOps, Right: LinkedOps> sealed::Ops for HBranch<Left, Right> {}

impl<Left: LinkedOps, Right: LinkedOps> LinkedOps for HBranch<Left, Right> {
    type Keys = HBranch<Left::Keys, Right::Keys>;

    const LEN: usize = Left::LEN + Right::LEN;

    fn into_keys(
        self,
        extras: &mut impl FnMut() -> Extra,
        entries: &mut impl FnMut(ErasedKey, Link),
    ) -> Self::Keys {
        HBranch {
            left: self.left.into_keys(extras, entries),
            right: self.right.into_keys(extras, entries),
        }
    }
}

impl<Op: OpCode + 'static> sealed::Keys for HLeaf<(Key<Op>, Link)> {}

impl<Op: OpCode + 'static> LinkedKeys for HLeaf<(Key<Op>, Link)> {
    type Ops = HLeaf<(Op, Link)>;

    fn into_ops(self) -> Self::Ops {
        let (key, link) = self.value;
        let BufResult(_, op) = key.take_result();
        HLeaf { value: (op, link) }
    }
}

impl<Left: LinkedKeys, Right: LinkedKeys> sealed::Keys for HBranch<Left, Right> {}

impl<Left: LinkedKeys, Right: LinkedKeys> LinkedKeys for HBranch<Left, Right> {
    type Ops = HBranch<Left::Ops, Right::Ops>;

    fn into_ops(self) -> Self::Ops {
        HBranch {
            left: self.left.into_ops(),
            right: self.right.into_ops(),
        }
    }
}

/// Inline capacity for linked submission bookkeeping.
#[cfg(io_uring)]
pub(crate) const LINKED_BATCH_INLINE: usize = 16;
