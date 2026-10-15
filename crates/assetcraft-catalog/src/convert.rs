//! Translation from Fab's wire format into the AssetCraft domain model.

use assetcraft_core::{
    AssetDetail, AssetFile, AssetFormat, AssetId, AssetSummary, Category, FormatType, LicenseType,
    Page, Rating, Seller, Tag, TagGroup,
};

use crate::wire;

/// Map a search response into a page of [`AssetSummary`].
pub fn page_from_search(results: wire::SearchResults) -> Page<AssetSummary> {
    let total = results.count;
    let next_cursor = results.cursors.and_then(|c| c.next);
    let items = results
        .results
        .into_iter()
        .map(summary_from_listing)
        .collect();
    Page::new(items, total, next_cursor)
}

/// Map a search listing into a domain summary.
pub fn summary_from_listing(listing: wire::SearchListing) -> AssetSummary {
    AssetSummary {
        id: AssetId::new(listing.uid),
        title: listing.title,
        listing_type: listing.listing_type,
        seller: listing.user.map(seller_from_user),
        category: listing.category.map(category_from_wire),
        tags: listing.tags.into_iter().map(tag_from_wire).collect(),
        rating: listing.ratings.and_then(rating_from_wire),
        is_free: listing.is_free,
        is_mature: listing.is_mature,
        published_at: listing.published_at,
        thumbnail_url: best_thumbnail(&listing.thumbnails),
    }
}

/// Map a detail response into a domain [`AssetDetail`].
pub fn detail_from_listing(listing: wire::ListingDetail) -> AssetDetail {
    let summary = AssetSummary {
        id: AssetId::new(listing.uid),
        title: listing.title,
        listing_type: listing.listing_type,
        seller: listing.user.map(seller_from_user),
        category: listing.category.map(category_from_wire),
        tags: listing.tags.into_iter().map(tag_from_wire).collect(),
        rating: listing.ratings.and_then(rating_from_wire),
        is_free: listing.is_free,
        is_mature: listing.is_mature,
        published_at: listing.published_at,
        thumbnail_url: best_thumbnail(&listing.thumbnails),
    };
    AssetDetail {
        summary,
        description: listing.description,
        review_count: listing.review_count,
        created_at: listing.created_at,
    }
}

/// Map a list of formats into domain types.
pub fn formats_from_wire(formats: Vec<wire::ListingFormat>) -> Vec<AssetFormat> {
    formats
        .into_iter()
        .map(|format| AssetFormat {
            format_type: format.asset_format_type.map(format_type_from_wire),
            files: format
                .files
                .into_iter()
                .map(|file| AssetFile {
                    id: file.uid,
                    name: file.name,
                    size_bytes: file.file_size,
                    asset_type: file.asset_type,
                    platforms: file.platforms_included,
                    engine_versions: file.engine_versions,
                })
                .collect(),
        })
        .collect()
}

/// Map license types into domain types.
pub fn licenses_from_wire(licenses: Vec<wire::LicenseType>) -> Vec<LicenseType> {
    licenses
        .into_iter()
        .map(|license| LicenseType {
            name: license.name,
            slug: license.slug,
            url: license.url,
        })
        .collect()
}

/// Map tag groups into domain types.
pub fn tag_groups_from_wire(groups: Vec<wire::TagGroup>) -> Vec<TagGroup> {
    groups
        .into_iter()
        .map(|group| TagGroup {
            id: group.uid,
            name: group.name,
            slug: group.slug,
            category: group.category,
            order: group.order,
            tags: group.tags.into_iter().map(tag_from_wire).collect(),
        })
        .collect()
}

fn seller_from_user(user: wire::User) -> Seller {
    Seller {
        id: user.seller_id.or(user.uid),
        name: user.seller_name,
        profile_url: user.profile_url,
    }
}

fn category_from_wire(category: wire::Category) -> Category {
    Category {
        id: category.uid,
        name: category.name,
        slug: category.slug,
        path: category.path,
    }
}

fn tag_from_wire(tag: wire::Tag) -> Tag {
    Tag {
        id: tag.uid,
        name: tag.name,
        slug: tag.slug,
    }
}

fn rating_from_wire(ratings: wire::Ratings) -> Option<Rating> {
    if ratings.average_rating.is_none() && ratings.total.is_none() {
        return None;
    }
    Some(Rating {
        average: ratings.average_rating,
        count: ratings.total,
    })
}

fn format_type_from_wire(format_type: wire::FormatType) -> FormatType {
    FormatType {
        code: format_type.code,
        name: format_type.name,
        icon: format_type.icon,
    }
}

/// Prefer a thumbnail explicitly typed `thumbnail`, else the first image.
fn best_thumbnail(thumbnails: &[wire::Thumbnail]) -> Option<String> {
    thumbnails
        .iter()
        .find(|thumb| thumb.thumbnail_type.as_deref() == Some("thumbnail"))
        .or_else(|| thumbnails.first())
        .and_then(|thumb| thumb.media_url.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefers_explicit_thumbnail_over_first_image() {
        let thumbnails = vec![
            wire::Thumbnail {
                media_url: Some("https://media/cover.jpg".to_string()),
                thumbnail_type: Some("gallery".to_string()),
                ..Default::default()
            },
            wire::Thumbnail {
                media_url: Some("https://media/thumb.jpg".to_string()),
                thumbnail_type: Some("thumbnail".to_string()),
                ..Default::default()
            },
        ];
        assert_eq!(
            best_thumbnail(&thumbnails).as_deref(),
            Some("https://media/thumb.jpg")
        );
    }

    #[test]
    fn falls_back_to_first_thumbnail() {
        let thumbnails = vec![wire::Thumbnail {
            media_url: Some("https://media/only.jpg".to_string()),
            ..Default::default()
        }];
        assert_eq!(
            best_thumbnail(&thumbnails).as_deref(),
            Some("https://media/only.jpg")
        );
    }

    #[test]
    fn no_thumbnail_yields_none() {
        assert!(best_thumbnail(&[]).is_none());
    }

    #[test]
    fn summary_maps_seller_category_and_rating() {
        let listing = wire::SearchListing {
            uid: "id-1".to_string(),
            title: Some("Cliff Rock".to_string()),
            listing_type: Some("3d-model".to_string()),
            user: Some(wire::User {
                seller_name: Some("Quixel".to_string()),
                seller_id: Some("o-quixel".to_string()),
                uid: Some("u-1".to_string()),
                ..Default::default()
            }),
            category: Some(wire::Category {
                uid: Some("c-1".to_string()),
                name: Some("Rocks".to_string()),
                slug: Some("rocks".to_string()),
                path: Some("nature/rocks".to_string()),
            }),
            ratings: Some(wire::Ratings {
                average_rating: Some(4.9),
                total: Some(100),
            }),
            ..Default::default()
        };

        let summary = summary_from_listing(listing);
        assert_eq!(summary.id.as_str(), "id-1");
        assert_eq!(summary.display_title(), "Cliff Rock");
        assert_eq!(summary.seller_name(), Some("Quixel"));
        // seller_id wins over the generic uid
        assert_eq!(
            summary.seller.as_ref().unwrap().id.as_deref(),
            Some("o-quixel")
        );
        assert_eq!(
            summary.category.as_ref().unwrap().name.as_deref(),
            Some("Rocks")
        );
        assert_eq!(summary.rating.as_ref().unwrap().average, Some(4.9));
    }

    #[test]
    fn empty_ratings_object_maps_to_none() {
        assert!(rating_from_wire(wire::Ratings::default()).is_none());
    }

    #[test]
    fn page_carries_the_next_cursor() {
        let results = wire::SearchResults {
            results: vec![wire::SearchListing {
                uid: "a".to_string(),
                ..Default::default()
            }],
            count: Some(42),
            cursors: Some(wire::Cursors {
                next: Some("cursor-2".to_string()),
                previous: None,
            }),
        };
        let page = page_from_search(results);
        assert_eq!(page.items.len(), 1);
        assert_eq!(page.total, Some(42));
        assert_eq!(page.next_cursor.as_deref(), Some("cursor-2"));
        assert!(page.has_more());
    }

    #[test]
    fn formats_map_files_and_type() {
        let formats = formats_from_wire(vec![wire::ListingFormat {
            asset_format_type: Some(wire::FormatType {
                code: Some("fbx".to_string()),
                name: Some("FBX".to_string()),
                icon: None,
            }),
            files: vec![wire::FormatFile {
                uid: Some("f1".to_string()),
                name: Some("asset.zip".to_string()),
                file_size: Some(2048),
                ..Default::default()
            }],
        }]);
        assert_eq!(formats[0].code(), Some("fbx"));
        assert_eq!(formats[0].files[0].size_bytes, Some(2048));
        assert_eq!(formats[0].files[0].name.as_deref(), Some("asset.zip"));
    }
}
