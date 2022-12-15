use rustc_data_structures::fx::FxHashMap;
use rustc_hir as hir;
use rustc_hir::def_id::DefId;
use rustc_hir::definitions::DefPathData;
use rustc_middle::ty::{self, TyCtxt};
use rustc_span::symbol::kw;

pub fn provide(providers: &mut ty::query::Providers) {
    *providers = ty::query::Providers {
        associated_item,
        associated_item_def_ids,
        associated_items,
        impl_item_implementor_ids,
        ..*providers
    };
}

fn associated_item_def_ids(tcx: TyCtxt<'_>, def_id: DefId) -> &[DefId] {
    let local_def_id = def_id.expect_local();
    let item = tcx.hir().expect_item(local_def_id);
    match item.kind {
        hir::ItemKind::Trait(.., ref trait_item_refs) => tcx.arena.alloc_from_iter(
            trait_item_refs
                .iter()
                .map(|trait_item_ref| trait_item_ref.id.owner_id.to_def_id())
                .chain(
                    trait_item_refs
                        .iter()
                        .filter(|trait_item_ref| {
                            matches!(trait_item_ref.kind, hir::AssocItemKind::Fn { .. })
                        })
                        .flat_map(|trait_item_ref| {
                            let trait_fn_def_id = trait_item_ref.id.owner_id.def_id.to_def_id();
                            tcx.assoc_items_for_rpits(trait_fn_def_id)
                        })
                        .map(|def_id| *def_id),
                ),
        ),
        hir::ItemKind::Impl(ref impl_) => tcx.arena.alloc_from_iter(
            impl_.items.iter().map(|impl_item_ref| impl_item_ref.id.owner_id.to_def_id()).chain(
                // FIXME what happens with default methods on traits?
                impl_.of_trait.iter().flat_map(|_| {
                    impl_
                        .items
                        .iter()
                        .filter(|impl_item_ref| {
                            matches!(impl_item_ref.kind, hir::AssocItemKind::Fn { .. })
                        })
                        .flat_map(|impl_item_ref| {
                            let trait_fn_def_id = impl_item_ref.trait_item_def_id.unwrap();
                            debug!("type_of: trait_fn_def_id={:?}", trait_fn_def_id);
                            tcx.assoc_items_for_rpits(trait_fn_def_id).iter().map(
                                |assoc_item_def_id| {
                                    debug!("type_of: assoc_item_def_id={:?}", assoc_item_def_id);
                                    // FIXME fix the span
                                    let span = tcx.def_span(assoc_item_def_id);
                                    let impl_trait_assoc_ty = tcx
                                        .at(span)
                                        .create_def(local_def_id, DefPathData::ImplTraitAssocTy);

                                    impl_trait_assoc_ty.associated_item(ty::AssocItem {
                                        name: kw::Empty,
                                        kind: ty::AssocKind::Type,
                                        def_id,
                                        trait_item_def_id: Some(*assoc_item_def_id),
                                        container: ty::ImplContainer,
                                        fn_has_self_parameter: false,
                                    });

                                    impl_trait_assoc_ty
                                        .type_of({
                                            match tcx.collect_trait_impl_trait_tys(impl_item_ref.id.owner_id.def_id) {
                                                Ok(map) => {
                                                    debug!("type_of: impl_item_ref.id.owner_id.def_id={:?}", impl_item_ref.id.owner_id.def_id);
                                                    debug!("type_of: impl_trait_assoc_ty.def_id().to_def_id()={:?}", impl_trait_assoc_ty.def_id().to_def_id());
                                                    debug!("type_of: collect_trait_impl_trait_tys map={:?}", map);
                                                    map[&assoc_item_def_id]
                                                }
                                                Err(_) => {
                                                    tcx.ty_error()
                                                }
                                            }
                                        });

                                    impl_trait_assoc_ty.def_id().to_def_id()
                                },
                            )
                        })
                }),
            ),
        ),
        hir::ItemKind::TraitAlias(..) => &[],
        _ => span_bug!(item.span, "associated_item_def_ids: not impl or trait"),
    }
}

fn associated_items(tcx: TyCtxt<'_>, def_id: DefId) -> ty::AssocItems<'_> {
    debug!("associated_items: def_id={:?}", def_id);
    let items = tcx.associated_item_def_ids(def_id).iter().map(|did| tcx.associated_item(*did));
    ty::AssocItems::new(items)
}

fn impl_item_implementor_ids(tcx: TyCtxt<'_>, impl_id: DefId) -> FxHashMap<DefId, DefId> {
    tcx.associated_items(impl_id)
        .in_definition_order()
        .filter_map(|item| item.trait_item_def_id.map(|trait_item| (trait_item, item.def_id)))
        .collect()
}

fn associated_item(tcx: TyCtxt<'_>, def_id: DefId) -> ty::AssocItem {
    debug!("associated_item: def_id={:?}", def_id);
    let id = tcx.hir().local_def_id_to_hir_id(def_id.expect_local());
    let parent_def_id = tcx.hir().get_parent_item(id);
    debug!("associated_item: parent_def_id={:?}", parent_def_id);
    let parent_item = tcx.hir().expect_item(parent_def_id.def_id);
    match parent_item.kind {
        hir::ItemKind::Impl(ref impl_) => {
            if let Some(impl_item_ref) =
                impl_.items.iter().find(|i| i.id.owner_id.to_def_id() == def_id)
            {
                let assoc_item = associated_item_from_impl_item_ref(impl_item_ref);
                debug_assert_eq!(assoc_item.def_id, def_id);
                return assoc_item;
            }
        }

        hir::ItemKind::Trait(.., ref trait_item_refs) => {
            if let Some(trait_item_ref) =
                trait_item_refs.iter().find(|i| i.id.owner_id.to_def_id() == def_id)
            {
                let assoc_item = associated_item_from_trait_item_ref(trait_item_ref);
                debug_assert_eq!(assoc_item.def_id, def_id);
                return assoc_item;
            }
        }

        _ => {}
    }

    span_bug!(
        parent_item.span,
        "unexpected parent of trait or impl item or item not found: {:?}",
        parent_item.kind
    )
}

fn associated_item_from_trait_item_ref(trait_item_ref: &hir::TraitItemRef) -> ty::AssocItem {
    let owner_id = trait_item_ref.id.owner_id;
    let (kind, has_self) = match trait_item_ref.kind {
        hir::AssocItemKind::Const => (ty::AssocKind::Const, false),
        hir::AssocItemKind::Fn { has_self } => (ty::AssocKind::Fn, has_self),
        hir::AssocItemKind::Type => (ty::AssocKind::Type, false),
    };

    ty::AssocItem {
        name: trait_item_ref.ident.name,
        kind,
        def_id: owner_id.to_def_id(),
        trait_item_def_id: Some(owner_id.to_def_id()),
        container: ty::TraitContainer,
        fn_has_self_parameter: has_self,
    }
}

fn associated_item_from_impl_item_ref(impl_item_ref: &hir::ImplItemRef) -> ty::AssocItem {
    let def_id = impl_item_ref.id.owner_id;
    let (kind, has_self) = match impl_item_ref.kind {
        hir::AssocItemKind::Const => (ty::AssocKind::Const, false),
        hir::AssocItemKind::Fn { has_self } => (ty::AssocKind::Fn, has_self),
        hir::AssocItemKind::Type => (ty::AssocKind::Type, false),
    };

    ty::AssocItem {
        name: impl_item_ref.ident.name,
        kind,
        def_id: def_id.to_def_id(),
        trait_item_def_id: impl_item_ref.trait_item_def_id,
        container: ty::ImplContainer,
        fn_has_self_parameter: has_self,
    }
}
