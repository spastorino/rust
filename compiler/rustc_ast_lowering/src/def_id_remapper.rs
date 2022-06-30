use rustc_ast::NodeId;
use rustc_data_structures::fx::FxHashMap;
use rustc_hir::def_id::LocalDefId;

use super::ResolverAstLowering;

pub struct DefIdRemapper<'a> {
    base_object: &'a mut dyn ResolverAstLowering,

    /// Used to map to the right def_id on generic parameters that are copied from the containing
    /// function into an inner type (e.g. impl trait).
    generics_def_id_map: Vec<FxHashMap<LocalDefId, LocalDefId>>,
}

impl<'a> DefIdRemapper<'a> {
    pub fn new(base_object: &'a mut dyn ResolverAstLowering) -> Self {
        Self { base_object, generics_def_id_map: Default::default() }
    }

    /// Pushes an empty map onto the stack.
    pub fn push_map(&mut self) {
        self.generics_def_id_map.push(Default::default());
    }

    pub fn pop_map(&mut self) {
        self.generics_def_id_map.pop();
    }

    pub fn is_empty_map(&self) -> bool {
        self.generics_def_id_map.is_empty()
    }

    /// Push a remapping into the top-most map. Panics if no map has been pushed.
    #[tracing::instrument(level = "debug", skip(self))]
    pub fn add_remapping(&mut self, from: LocalDefId, to: LocalDefId) {
        self.generics_def_id_map.last_mut().expect("no map pushed").insert(from, to);
    }

    pub fn take_maps(&mut self) -> Vec<FxHashMap<LocalDefId, LocalDefId>> {
        std::mem::take(&mut self.generics_def_id_map)
    }

    pub fn restore_maps(&mut self, v: Vec<FxHashMap<LocalDefId, LocalDefId>>) {
        self.generics_def_id_map = v;
    }

    pub fn remap(&self, mut local_def_id: LocalDefId) -> LocalDefId {
        for map in &self.generics_def_id_map {
            if let Some(r) = map.get(&local_def_id) {
                debug!("def_id_remapper: remapping from `{local_def_id:?}` to `{r:?}`");
                local_def_id = *r;
            } else {
                debug!("def_id_remapper: no remapping for `{local_def_id:?}` found in map");
            }
        }

        local_def_id
    }
}

impl ResolverAstLowering for DefIdRemapper<'_> {
    fn def_key(&self, id: rustc_hir::def_id::DefId) -> rustc_hir::definitions::DefKey {
        self.base_object.def_key(id)
    }

    fn def_span(&self, id: LocalDefId) -> rustc_span::Span {
        self.base_object.def_span(id)
    }

    fn item_generics_num_lifetimes(&self, def: rustc_hir::def_id::DefId) -> usize {
        self.base_object.item_generics_num_lifetimes(def)
    }

    fn legacy_const_generic_args(&mut self, expr: &rustc_ast::Expr) -> Option<Vec<usize>> {
        self.base_object.legacy_const_generic_args(expr)
    }

    fn get_partial_res(&self, id: NodeId) -> Option<rustc_hir::def::PartialRes> {
        self.base_object.get_partial_res(id)
    }

    fn get_import_res(
        &self,
        id: NodeId,
    ) -> rustc_hir::def::PerNS<Option<rustc_hir::def::Res<NodeId>>> {
        self.base_object.get_import_res(id)
    }

    fn get_label_res(&self, id: NodeId) -> Option<NodeId> {
        self.base_object.get_label_res(id)
    }

    fn get_lifetime_res(&self, id: NodeId) -> Option<crate::LifetimeRes> {
        self.base_object.get_lifetime_res(id).map(|res| {
            if let crate::LifetimeRes::Param { param, binder } = res {
                crate::LifetimeRes::Param {
                    param: self.remap(param),
                    binder,
                }
            } else {
                res
            }
        })
    }

    fn take_extra_lifetime_params(
        &mut self,
        id: NodeId,
    ) -> Vec<(rustc_span::symbol::Ident, NodeId, crate::LifetimeRes)> {
        self.base_object.take_extra_lifetime_params(id).into_iter().map(|(ident, node_id, res)| {
            if let crate::LifetimeRes::Param { param, binder } = res {
                (ident, node_id, crate::LifetimeRes::Param {
                    param: self.remap(param),
                    binder,
                })
            } else {
                (ident, node_id, res)
            }
        }).collect()
    }

    fn create_stable_hashing_context(&self) -> rustc_query_system::ich::StableHashingContext<'_> {
        self.base_object.create_stable_hashing_context()
    }

    fn definitions(&self) -> &rustc_hir::definitions::Definitions {
        self.base_object.definitions()
    }

    fn next_node_id(&mut self) -> NodeId {
        self.base_object.next_node_id()
    }

    fn take_trait_map(&mut self, node: NodeId) -> Option<Vec<rustc_hir::TraitCandidate>> {
        self.base_object.take_trait_map(node)
    }

    fn opt_local_def_id(&self, node: NodeId) -> Option<LocalDefId> {
        self.base_object.opt_local_def_id(node).map(|local_def_id| self.remap(local_def_id))
    }

    fn local_def_id(&self, node: NodeId) -> LocalDefId {
        self.opt_local_def_id(node).unwrap()
    }

    fn def_path_hash(&self, def_id: rustc_hir::def_id::DefId) -> rustc_hir::def_id::DefPathHash {
        self.base_object.def_path_hash(def_id)
    }

    fn create_def(
        &mut self,
        parent: LocalDefId,
        node_id: rustc_ast::NodeId,
        data: rustc_hir::definitions::DefPathData,
        expn_id: rustc_span::ExpnId,
        span: rustc_span::Span,
    ) -> LocalDefId {
        self.base_object.create_def(parent, node_id, data, expn_id, span)
    }

    fn decl_macro_kind(&self, def_id: LocalDefId) -> rustc_span::MacroKind {
        self.base_object.decl_macro_kind(def_id)
    }
}
